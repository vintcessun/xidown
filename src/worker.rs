//! 单部戏的执行流程：下载分P -> 上传 -> 投稿/追加 -> 记台账 -> 删临时文件。

use crate::bili::{self, ArchiveMeta};
use crate::catalog::Task;
use crate::config::{DESC, KEYWORD, SOURCE, Settings, TAG, TID, truncate_title};
use crate::ledger::{Entry, Ledger, now_ts};
use anyhow::{Context, Result, anyhow};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::{info, warn};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use xmtv_api::VideoUrl;

/// 稿件被打回后最多重新投几次，避免无限循环刷稿。
const MAX_RESUBMIT: usize = 1;

pub struct Ctx {
    pub settings: Settings,
    pub ledger: Arc<Ledger>,
    pub multi: Arc<MultiProgress>,
}

pub async fn run_task(ctx: Arc<Ctx>, task: Task) -> Result<()> {
    let mut bv = task.bv.clone();
    let mut resubmits = 0usize;

    loop {
        match run_once(&ctx, &task, &mut bv).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                // 只有"稿件已经废了"这一种情况值得重新投一个，其它错误直接上报
                if bv.is_empty() || resubmits >= MAX_RESUBMIT {
                    return Err(e);
                }
                match bili::archive_state(&bv).await {
                    Ok(state) if bili::state_is_dead(state) => {
                        warn!("{} 的稿件 {bv} 状态 {state}，重新投一个", task.title);
                        ctx.ledger.forget_archive(&bv).await?;
                        bv.clear();
                        resubmits += 1;
                    }
                    Ok(state) => {
                        return Err(e.context(format!("稿件 {bv} 状态正常({state})，但处理失败")));
                    }
                    Err(se) => return Err(e.context(format!("并且查询稿件状态也失败: {se}"))),
                }
            }
        }
    }
}

async fn run_once(ctx: &Ctx, task: &Task, bv: &mut String) -> Result<()> {
    let meta = ArchiveMeta {
        title: format!("{} {KEYWORD}", task.title),
        tid: TID,
        tag: TAG.to_string(),
        source: SOURCE.to_string(),
        desc: DESC.to_string(),
        copyright: 2,
    };

    let mut parts = task.parts.iter();

    // 还没有稿件时，第一个分P 要用来新建稿件
    if bv.is_empty() {
        let first = parts
            .next()
            .ok_or_else(|| anyhow!("{} 没有需要上传的分P", task.title))?;
        let (video, path) = fetch_and_upload(ctx, first).await?;
        let part_title = video.title.clone().unwrap_or_default();
        let new_bv = bili::submit_new(&meta, vec![video]).await?;
        cleanup(&path).await;
        record(ctx, task, first, &new_bv, &part_title).await?;
        *bv = new_bv;
    }

    for part in parts {
        // 台账可能在这次运行里刚被别的分支写过，再查一次
        if ctx
            .ledger
            .contains(&crate::ledger::part_key(part), Some(bv))
            .await
            .is_some()
        {
            info!("{} 已在台账中，跳过", part.name);
            continue;
        }
        let (video, path) = fetch_and_upload(ctx, part).await?;
        let part_title = video.title.clone().unwrap_or_default();
        bili::append_parts(bv, vec![video]).await?;
        cleanup(&path).await;
        record(ctx, task, part, bv, &part_title).await?;
    }
    Ok(())
}

async fn record(
    ctx: &Ctx,
    task: &Task,
    part: &VideoUrl,
    bv: &str,
    part_title: &str,
) -> Result<()> {
    ctx.ledger
        .record(
            crate::ledger::part_key(part),
            Entry {
                bv: bv.to_string(),
                part_title: part_title.to_string(),
                title: task.title.clone(),
                uploaded_at: now_ts(),
            },
        )
        .await
}

/// 下载 + 上传，返回可以塞进稿件的分P 和本地临时文件路径。
async fn fetch_and_upload(
    ctx: &Ctx,
    part: &VideoUrl,
) -> Result<(biliup::bilibili::Video, PathBuf)> {
    let stem = part.safe_file_stem();
    let path = ctx.settings.work_dir.join(format!("{stem}.mp4"));

    let src = xmtv_api::get_video_url(&part.url).await?;
    download(&src, &path, part, Some(&ctx.multi)).await?;

    let part_title = truncate_title(&part.name);
    let video = bili::upload_part(
        &path,
        &part_title,
        ctx.settings.upload_limit,
        Some(&ctx.multi),
    )
    .await;

    match video {
        Ok(v) => Ok((v, path)),
        Err(e) => {
            // 上传失败时也要把临时文件清掉，否则跑一晚上磁盘就满了
            cleanup(&path).await;
            Err(e)
        }
    }
}

async fn cleanup(path: &Path) {
    if let Err(e) = tokio::fs::remove_file(path).await {
        warn!("删除临时文件 {} 失败: {e}", path.display());
    }
}

/// 下载到 `xxx.mp4.part` 再改名，半个文件不会被当成下载完成的结果。
///
/// 一集正片有好几百兆，中途断流是常态（实测下到 263MB 时连接被掐断，
/// 紧接着 CDN 还会拒连一段时间）。所以：
/// - 失败时**保留** `.part`，下次用 Range 断点续传，不重头再来；
/// - 退避给得足够长，让 CDN 的冷却期过去。
async fn download(
    url: &str,
    dest: &Path,
    part: &VideoUrl,
    multi: Option<&MultiProgress>,
) -> Result<()> {
    const ATTEMPTS: usize = 6;
    let tmp = dest.with_extension("mp4.part");
    let mut last: Option<anyhow::Error> = None;

    for i in 0..ATTEMPTS {
        let have = tokio::fs::metadata(&tmp).await.map(|m| m.len()).unwrap_or(0);
        match download_once(url, &tmp, have, part, multi).await {
            Ok(()) => {
                tokio::fs::rename(&tmp, dest)
                    .await
                    .context(format!("重命名 {} 失败", tmp.display()))?;
                return Ok(());
            }
            Err(e) => {
                let have = tokio::fs::metadata(&tmp).await.map(|m| m.len()).unwrap_or(0);
                warn!(
                    "下载 {} 第 {} 次失败（已下 {:.1} MB，下次断点续传）: {e}",
                    part.name,
                    i + 1,
                    have as f64 / 1024. / 1024.
                );
                last = Some(e);
                if i + 1 < ATTEMPTS {
                    // 15/30/60/120/120 秒——CDN 掐断之后短间隔重连只会一直被拒
                    tokio::time::sleep(Duration::from_secs(15 * 2u64.pow(i.min(3) as u32))).await;
                }
            }
        }
    }
    // 彻底放弃了才清掉半成品，免得占着磁盘
    tokio::fs::remove_file(&tmp).await.ok();
    Err(last.unwrap_or_else(|| anyhow!("下载 {} 失败", part.name)))
}

/// 从 `resume_from` 字节处继续下载。返回 Ok 表示文件已经完整。
async fn download_once(
    url: &str,
    tmp: &Path,
    resume_from: u64,
    part: &VideoUrl,
    multi: Option<&MultiProgress>,
) -> Result<()> {
    // 复用 xmtv_api 里的连接池，别每次都新建一个 Client
    let mut req = xmtv_api::client().get(url);
    if resume_from > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
    }
    let mut source = req
        .send()
        .await?
        .error_for_status()
        .context(format!("下载 {url} 被服务端拒绝"))?;

    let status = source.status();
    // 206 说明服务端接受了续传；返回 200 表示它忽略了 Range，只能从头来过
    let resuming = resume_from > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
    if resume_from > 0 && !resuming {
        warn!("服务端不支持断点续传（返回 {status}），从头开始下载");
    }

    let total_size = if resuming {
        // Content-Range: bytes 100-999/1000
        source
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next().map(str::to_string))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or_else(|| resume_from + source.content_length().unwrap_or(0))
    } else {
        source.content_length().unwrap_or(0)
    };

    let pb = ProgressBar::new(total_size);
    let pb = match multi {
        Some(m) => m.add(pb),
        None => pb,
    };
    pb.set_style(ProgressStyle::default_bar().template(&format!(
        "{{spinner:.green}} 下载 {} [{{elapsed_precise}}] [{{wide_bar:.cyan/blue}}] {{bytes}}/{{total_bytes}} ({{bytes_per_sec}}, {{eta}})",
        bili::short_label(&part.name)
    ))?);

    let mut written = if resuming { resume_from } else { 0 };
    pb.set_position(written);

    let result = async {
        let file = if resuming {
            tokio::fs::OpenOptions::new().append(true).open(tmp).await
        } else {
            tokio::fs::File::create(tmp).await
        }
        .context(format!("打开 {} 失败", tmp.display()))?;
        let mut file = tokio::io::BufWriter::new(file);
        while let Some(chunk) = source.chunk().await? {
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
            pb.inc(chunk.len() as u64);
        }
        file.flush().await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    pb.finish_and_clear();
    // 出错也要保证已经收到的字节落盘，下次才能接着下
    result?;

    // 截断的下载会让上传出去的视频缺一段，这里必须拦住
    if total_size != 0 && written != total_size {
        return Err(anyhow!(
            "下载不完整: 期望 {total_size} 字节，实际 {written} 字节"
        ));
    }
    if written == 0 {
        return Err(anyhow!("下载到的文件是空的"));
    }
    info!("下载完成 {} ({} 字节)", tmp.display(), written);
    Ok(())
}
