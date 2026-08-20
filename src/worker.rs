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
async fn download(
    url: &str,
    dest: &Path,
    part: &VideoUrl,
    multi: Option<&MultiProgress>,
) -> Result<()> {
    const ATTEMPTS: usize = 4;
    let tmp = dest.with_extension("mp4.part");
    let mut last: Option<anyhow::Error> = None;

    for i in 0..ATTEMPTS {
        match download_once(url, &tmp, part, multi).await {
            Ok(()) => {
                tokio::fs::rename(&tmp, dest)
                    .await
                    .context(format!("重命名 {} 失败", tmp.display()))?;
                return Ok(());
            }
            Err(e) => {
                warn!("下载 {} 第 {} 次失败: {e}", part.name, i + 1);
                tokio::fs::remove_file(&tmp).await.ok();
                last = Some(e);
                if i + 1 < ATTEMPTS {
                    tokio::time::sleep(Duration::from_secs(2u64.pow(i as u32))).await;
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("下载 {} 失败", part.name)))
}

async fn download_once(
    url: &str,
    tmp: &Path,
    part: &VideoUrl,
    multi: Option<&MultiProgress>,
) -> Result<()> {
    // 复用 xmtv_api 里的连接池，别每次都新建一个 Client
    let mut source = xmtv_api::client()
        .get(url)
        .send()
        .await?
        .error_for_status()
        .context(format!("下载 {url} 被服务端拒绝"))?;

    let total_size = source
        .content_length()
        .or_else(|| {
            source
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0);

    let pb = ProgressBar::new(total_size);
    let pb = match multi {
        Some(m) => m.add(pb),
        None => pb,
    };
    pb.set_style(ProgressStyle::default_bar().template(&format!(
        "{{spinner:.green}} 下载 {} [{{elapsed_precise}}] [{{wide_bar:.cyan/blue}}] {{bytes}}/{{total_bytes}} ({{bytes_per_sec}}, {{eta}})",
        bili::short_label(&part.name)
    ))?);

    let mut written = 0u64;
    {
        let file = tokio::fs::File::create(tmp)
            .await
            .context(format!("创建 {} 失败", tmp.display()))?;
        let mut file = tokio::io::BufWriter::new(file);
        while let Some(chunk) = source.chunk().await? {
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
            pb.inc(chunk.len() as u64);
        }
        file.flush().await?;
    }
    pb.finish_and_clear();

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
