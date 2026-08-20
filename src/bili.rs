//! 对 biliup 的一层薄封装：登录、查稿件、上传分P、投稿、追加分P。
//!
//! 相比旧版主要有三点不同：
//! 1. 所有 `loop { if let Ok(..) }` 的空转重试都换成了带上限的指数退避，
//!    并且对 b 站的限流错误（code 601）单独放长等待时间；
//! 2. bv 号从 `Value::to_string()` 改成 `as_str()`——前者会把 JSON 的引号一起带上，
//!    于是后续所有拿这个 bv 去查询/追加的调用都在用 `"BVxxx"` 这种带引号的串；
//! 3. 分P 标题由我们自己显式指定，而不是让 biliup 从文件名推断，
//!    这样本地台账里记的标题和 b 站上的完全一致，去重才靠得住。

use crate::config::{ARCHIVE_STATUS, COOKIE_FILE, PROXY, truncate_title};
use anyhow::{Context, Result, anyhow};
use biliup::bilibili::{Archive, BiliBili, Studio, Vid, Video};
use biliup::client::StatelessClient;
use biliup::credential::login_by_cookies;
use biliup::error::Kind;
use biliup::uploader::{VideoFile, line};
use bytes::{Buf, Bytes};
use futures::{Stream, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::{debug, info, warn};
use reqwest::Body;
use serde_json::Value;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

static BILI: OnceCell<Arc<BiliBili>> = OnceCell::const_new();

/// 登录（只做一次，后续复用同一个客户端）。
pub async fn bili() -> Result<Arc<BiliBili>> {
    BILI.get_or_try_init(|| async {
        info!("使用 {COOKIE_FILE} 登录 b 站");
        let b = login_by_cookies(COOKIE_FILE, PROXY)
            .await
            .with_context(|| format!("使用 {COOKIE_FILE} 登录失败，cookie 可能已经过期"))?;
        Ok::<_, anyhow::Error>(Arc::new(b))
    })
    .await
    .cloned()
}

/// 启动时先确认账号可用，免得跑到一半才发现 cookie 过期。
pub async fn probe_login() -> Result<String> {
    let b = bili().await?;
    let info = b.my_info().await.map_err(|e| anyhow!("查询账号信息失败: {e}"))?;
    let data = &info["data"];
    let name = data["name"].as_str().unwrap_or_default().to_string();
    let mid = data["mid"].as_i64().unwrap_or_default();
    if name.is_empty() || mid == 0 {
        return Err(anyhow!("账号信息为空，cookie 大概率已失效: {info}"));
    }
    info!("登录成功: {name} (mid={mid})");
    Ok(name)
}

/// 当前账号的所有稿件（含审核中、未通过的）。
///
/// 旧版是去翻空间动态搜索接口，那个接口只能看到已发布的稿件，而且索引有延迟，
/// 于是刚投出去的稿件下次运行时会被当成"还没传过"再投一遍。这里换成投稿中心的
/// 接口，它是权威数据源。
pub async fn list_archives() -> Result<Vec<Archive>> {
    let b = bili().await?;
    let archives = retry("获取稿件列表", 5, || {
        let b = b.clone();
        async move { b.recent_archives(ARCHIVE_STATUS, 1, None).await }
    })
    .await?;
    info!("获取到 {} 个稿件", archives.len());
    Ok(archives)
}

/// 按关键词搜稿件。
///
/// 投稿前确认"这部戏是不是已经有稿件了"只需要这一个请求，
/// 而翻完整的稿件列表要二十多页，很容易触发 -702 限流。
pub async fn search_archives(keyword: &str) -> Result<Vec<(String, String)>> {
    let b = bili().await?;
    let cookie = b
        .login_info
        .cookie_info
        .get("cookies")
        .and_then(|c: &Value| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| match (c["name"].as_str(), c["value"].as_str()) {
                    (Some(n), Some(v)) => Some(format!("{n}={v}")),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default();

    let json: Value = retry_http("搜索稿件", 4, || {
        let cookie = cookie.clone();
        async move {
            reqwest::Client::new()
                .get("https://member.bilibili.com/x/web/archives")
                .query(&[
                    ("status", ARCHIVE_STATUS),
                    ("pn", "1"),
                    ("ps", "50"),
                    ("keyword", keyword),
                ])
                .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/63.0.3239.108")
                .header("Cookie", cookie)
                .timeout(Duration::from_secs(60))
                .send()
                .await?
                .json::<Value>()
                .await
                .map_err(Kind::from)
        }
    })
    .await?;

    if json["code"].as_i64() != Some(0) {
        return Err(anyhow!("搜索稿件失败: {json}"));
    }
    Ok(json["data"]["arc_audits"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let arc = &a["Archive"];
                    Some((
                        arc["bvid"].as_str()?.to_string(),
                        arc["title"].as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default())
}

/// 稿件里已有的分P 标题。
pub async fn part_titles(bv: &str) -> Result<Vec<String>> {
    let json = video_data(bv).await?;
    Ok(json["videos"]
        .as_array()
        .map(|vs| {
            vs.iter()
                .filter_map(|v| v["title"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

pub async fn video_data(bv: &str) -> Result<Value> {
    let b = bili().await?;
    let vid = Vid::Bvid(bv.to_owned());
    retry("查询稿件", 5, || {
        let b = b.clone();
        let vid = vid.clone();
        async move { b.video_data(&vid, PROXY).await }
    })
    .await
}

/// 稿件状态码。0 正常，负数一般是审核未通过 / 被锁定。
pub async fn archive_state(bv: &str) -> Result<i64> {
    let json = video_data(bv).await?;
    json["archive"]["state"]
        .as_i64()
        .ok_or_else(|| anyhow!("稿件 {bv} 的 state 字段无法解析: {}", json["archive"]))
}

/// 这些状态说明稿件已经废了，只能重新投一个。
pub fn state_is_dead(state: i64) -> bool {
    matches!(state, -2 | -3 | -4 | -5 | -12 | -16 | -100)
}

/// 上传一个文件，返回可以塞进 `Studio.videos` 的分P。
///
/// 重试放在这一层：外层重试意味着要把几百兆的文件重新下一遍，
/// 而这里失败时本地文件还在，直接重传就行。
pub async fn upload_part(
    path: &Path,
    part_title: &str,
    limit: usize,
    multi: Option<&MultiProgress>,
) -> Result<Video> {
    const ATTEMPTS: usize = 4;
    let mut last: Option<Kind> = None;

    for i in 0..ATTEMPTS {
        match upload_part_once(path, part_title, limit, multi).await {
            Ok(mut video) => {
                // 显式指定分P 标题，不让 biliup 从文件名去猜
                video.title = Some(truncate_title(part_title));
                return Ok(video);
            }
            Err(e) => {
                let wait = match &e {
                    Kind::RateLimit { code, message } => {
                        warn!("上传被限流(code {code}): {message}，等待 5 分钟后重试");
                        Duration::from_secs(300)
                    }
                    _ => {
                        warn!("上传 {} 第 {} 次失败: {e}", path.display(), i + 1);
                        Duration::from_secs(10 * 2u64.pow(i.min(4) as u32))
                    }
                };
                last = Some(e);
                if i + 1 < ATTEMPTS {
                    tokio::time::sleep(wait).await;
                }
            }
        }
    }
    Err(anyhow!(
        "上传 {} 重试 {ATTEMPTS} 次仍然失败: {}",
        path.display(),
        last.map(|e| e.to_string()).unwrap_or_default()
    ))
}

async fn upload_part_once(
    path: &Path,
    part_title: &str,
    limit: usize,
    multi: Option<&MultiProgress>,
) -> std::result::Result<Video, Kind> {
    let b = bili().await.map_err(|e| Kind::Custom(e.to_string()))?;
    let client = StatelessClient::default();
    let line = line::bda2();

    let video_file = VideoFile::new(path)?;
    let total_size = video_file.total_size;
    let file_name = video_file.file_name.clone();

    let parcel = line.pre_upload(&b, video_file).await?;

    let pb = ProgressBar::new(total_size);
    let pb = match multi {
        Some(m) => m.add(pb),
        None => pb,
    };
    let tpl = format!(
        "{{spinner:.green}} 上传 {} [{{elapsed_precise}}] [{{wide_bar:.cyan/blue}}] {{bytes}}/{{total_bytes}} ({{bytes_per_sec}}, {{eta}})",
        short_label(part_title)
    );
    pb.set_style(
        ProgressStyle::default_bar()
            .template(&tpl)
            .map_err(|e| Kind::Custom(e.to_string()))?,
    );

    let instant = Instant::now();
    let result = parcel
        .upload(client, limit, |vs| {
            vs.map(|chunk| {
                let pb = pb.clone();
                let chunk = chunk?;
                let len = chunk.len();
                Ok((Progressbar::new(chunk, pb), len))
            })
        })
        .await;
    // 失败时也要把进度条收掉，否则重试会在屏幕上叠一堆卡死的条
    pb.finish_and_clear();
    let video = result?;

    let cost = instant.elapsed().as_secs_f64();
    info!(
        "上传完成 {file_name} 耗时 {cost:.1}s，平均 {:.2} MB/s",
        total_size as f64 / 1024. / 1024. / cost.max(0.001)
    );
    Ok(video)
}

pub struct ArchiveMeta {
    pub title: String,
    pub tid: u16,
    pub tag: String,
    pub source: String,
    pub desc: String,
    pub copyright: u8,
}

/// 新投一个稿件，返回干净的 bv 号（不带引号）。
pub async fn submit_new(meta: &ArchiveMeta, videos: Vec<Video>) -> Result<String> {
    let b = bili().await?;
    let studio = Studio::builder()
        .copyright(meta.copyright)
        .source(meta.source.clone())
        .tid(meta.tid)
        .cover(String::new())
        .title(truncate_title(&meta.title))
        .desc(meta.desc.clone())
        .dynamic(String::new())
        .tag(meta.tag.clone())
        .videos(videos)
        .no_reprint(0)
        .dolby(0)
        .charging_pay(0)
        .up_selection_reply(false)
        .up_close_reply(false)
        .up_close_danmu(false)
        .build();

    let wanted = truncate_title(&meta.title);
    const ATTEMPTS: usize = 4;
    let mut last = String::new();

    // 投之前先确认这部戏是不是已经有稿件了。稿件列表可能是缓存的、也可能因为
    // 标题对不上而没匹配到，这一次针对性查询是"不重复投稿"的最后一道保险。
    match find_archive_by_title(&wanted).await {
        Ok(Some(bv)) => {
            warn!("{wanted} 已经存在稿件 {bv}，不再新投一个");
            return Ok(bv);
        }
        Ok(None) => {}
        Err(e) => warn!("投稿前确认稿件是否已存在失败({e})，继续投稿"),
    }

    for i in 0..ATTEMPTS {
        // 重试前再确认一次上一次是不是其实已经投成功了：服务端处理完但响应超时的话，
        // 盲目重试就会平白多出一个稿件——这正是我们要消灭的重复来源。
        if i > 0 {
            match find_archive_by_title(&wanted).await {
                Ok(Some(bv)) => {
                    warn!("{wanted} 其实已经投稿成功了({bv})，不再重复提交");
                    return Ok(bv);
                }
                Ok(None) => {}
                Err(e) => warn!("重试前确认稿件是否已存在失败({e})，继续重试投稿"),
            }
        }

        match b.submit_by_app(&studio, PROXY).await {
            Ok(resp) => {
                let dump = format!("{resp:?}");
                let data = resp
                    .data
                    .ok_or_else(|| anyhow!("投稿返回里没有 data 字段: {dump}"))?;
                // 旧版这里是 `data["bvid"].to_string()`，会把 JSON 的引号一起带进来，
                // 后面拿这个 bv 去查询/追加全都是错的。
                let bv = data["bvid"]
                    .as_str()
                    .ok_or_else(|| anyhow!("投稿返回里没有 bvid: {data}"))?
                    .to_string();
                info!("投稿成功 {} => {bv}", meta.title);
                return Ok(bv);
            }
            Err(e) => {
                let wait = match &e {
                    Kind::RateLimit { code, message } => {
                        warn!("投稿被限流(code {code}): {message}，等待 5 分钟后重试");
                        Duration::from_secs(300)
                    }
                    _ => {
                        warn!("投稿第 {} 次失败: {e}", i + 1);
                        Duration::from_secs(10 * 2u64.pow(i.min(4) as u32))
                    }
                };
                last = e.to_string();
                if i + 1 < ATTEMPTS {
                    tokio::time::sleep(wait).await;
                }
            }
        }
    }
    Err(anyhow!("投稿 {wanted} 重试 {ATTEMPTS} 次仍然失败: {last}"))
}

/// 按稿件标题精确查找已有稿件，用来判断某次投稿是不是其实已经成功了。
/// 走关键词搜索而不是翻整个稿件列表，只要一个请求。
async fn find_archive_by_title(title: &str) -> Result<Option<String>> {
    let hits = search_archives(title).await?;
    Ok(hits
        .into_iter()
        .find(|(_, t)| t == title)
        .map(|(bv, _)| bv))
}

/// 等到刚投出去的稿件能被查询为止。
///
/// 投稿返回 bv 之后，稿件要过一会儿才会出现在查询接口里，
/// 这段时间直接去追加分P 会失败。
pub async fn wait_until_queryable(bv: &str, max_wait: Duration) -> Result<()> {
    let deadline = Instant::now() + max_wait;
    let mut wait = Duration::from_secs(5);
    loop {
        match video_data(bv).await {
            Ok(_) => {
                info!("稿件 {bv} 已可查询");
                return Ok(());
            }
            Err(e) if Instant::now() + wait < deadline => {
                info!("稿件 {bv} 还查不到（{e}），{wait:?} 后重试");
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(30));
            }
            Err(e) => return Err(e.context(format!("等待稿件 {bv} 可查询超时"))),
        }
    }
}

/// 往已有稿件追加分P。
pub async fn append_parts(bv: &str, mut videos: Vec<Video>) -> Result<()> {
    let b = bili().await?;
    let vid = Vid::Bvid(bv.to_owned());
    let mut studio = retry("获取稿件信息", 5, || {
        let b = b.clone();
        let vid = vid.clone();
        async move { b.studio_data(&vid, PROXY).await }
    })
    .await?;

    // 追加之前再确认一次：万一这个分P 已经在稿件里了就别重复加
    let existing: Vec<_> = studio
        .videos
        .iter()
        .filter_map(|v| v.title.clone())
        .collect();
    videos.retain(|v| match &v.title {
        Some(t) if existing.contains(t) => {
            warn!("分P {t} 已经在 {bv} 里了，跳过追加");
            false
        }
        _ => true,
    });
    if videos.is_empty() {
        return Ok(());
    }

    studio.videos.append(&mut videos);
    let ret = retry("修改稿件", 5, || {
        let b = b.clone();
        let studio = &studio;
        async move { b.edit_by_web(studio).await }
    })
    .await?;
    debug!("追加分P 返回 {ret}");
    Ok(())
}

/// 带指数退避的重试。
///
/// 遇到 b 站限流（code 601）时等得更久——旧版在这种情况下会原地疯狂重试，
/// 反而让限流更难解除。
async fn retry<T, F, Fut>(what: &str, attempts: usize, f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, Kind>>,
{
    retry_http(what, attempts, f).await
}

/// b 站的限流有两种表现：上传接口的 `Kind::RateLimit`(601)，
/// 以及投稿中心接口在 body 里返回的 `code: -702 请求频率过高`。
/// 后者在类型上就是一个普通错误，只能靠文本认出来——但它同样需要长等待，
/// 用几秒的退避去重试只会一直撞在限流上。
fn rate_limit_wait(e: &Kind) -> Option<Duration> {
    match e {
        Kind::RateLimit { .. } => Some(Duration::from_secs(300)),
        _ => {
            let msg = e.to_string();
            (msg.contains("-702") || msg.contains("请求频率过高"))
                .then(|| Duration::from_secs(120))
        }
    }
}

async fn retry_http<T, F, Fut>(what: &str, attempts: usize, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, Kind>>,
{
    let mut last: Option<Kind> = None;
    for i in 0..attempts {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                let wait = match rate_limit_wait(&e) {
                    Some(w) => {
                        warn!("{what} 被限流，等待 {w:?} 后重试: {e}");
                        w
                    }
                    None => {
                        warn!("{what} 第 {} 次失败: {e}", i + 1);
                        Duration::from_secs(2u64.pow(i.min(5) as u32))
                    }
                };
                last = Some(e);
                if i + 1 < attempts {
                    tokio::time::sleep(wait).await;
                }
            }
        }
    }
    Err(anyhow!(
        "{what} 重试 {attempts} 次仍然失败: {}",
        last.map(|e| e.to_string()).unwrap_or_default()
    ))
}

/// 进度条上只显示剧目名那一段，太长的话会把整行挤爆。
pub fn short_label(name: &str) -> String {
    let head = name
        .split(crate::config::KEYWORD)
        .next()
        .unwrap_or(name)
        .trim();
    let head = if head.is_empty() { name } else { head };
    head.chars().take(16).collect()
}

impl From<Progressbar> for Body {
    fn from(async_stream: Progressbar) -> Self {
        Body::wrap_stream(async_stream)
    }
}

#[derive(Clone)]
struct Progressbar {
    bytes: Bytes,
    pb: ProgressBar,
}

impl Progressbar {
    pub fn new(bytes: Bytes, pb: ProgressBar) -> Self {
        Self { bytes, pb }
    }

    pub fn progress(&mut self) -> Option<Bytes> {
        let pb = &self.pb;
        let content_bytes = &mut self.bytes;
        let n = content_bytes.remaining();

        const PC: usize = 4096;
        if n == 0 {
            None
        } else if n < PC {
            pb.inc(n as u64);
            Some(content_bytes.copy_to_bytes(n))
        } else {
            pb.inc(PC as u64);
            Some(content_bytes.copy_to_bytes(PC))
        }
    }
}

impl Stream for Progressbar {
    type Item = Result<Bytes>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match self.progress() {
            None => Poll::Ready(None),
            Some(s) => Poll::Ready(Some(Ok(s))),
        }
    }
}
