use anyhow::Result;
use async_static::async_static;
use biliup::bilibili::{BiliBili, Studio, Vid, Video};
use biliup::client::StatelessClient;
use biliup::credential::{login_by_cookies, Credential};
use biliup::uploader::{line, VideoFile};
use bytes::{Buf, Bytes};
use futures::{Stream, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use lazy_static::lazy_static;
use log::debug;
use reqwest::Body;
use serde_json::Value;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::Instant;

const COOKIE_FILE: &str = r#"cookies.json"#;
const PROXY: Option<&str> = None;

lazy_static! {
    static ref CLIENT: Arc<Credential> = Arc::new(Credential::new(PROXY));
}
async_static! {
    static ref LOGININFO: Arc<BiliBili> = Arc::new(login_by_cookies(COOKIE_FILE,PROXY).await.unwrap());
}

pub struct VideoInfo {
    pub title: String,  //标题
    pub copyright: u8,  //1自制 2转载
    pub source: String, //来源
    pub tag: String,    //用逗号分割
    pub tid: u16,       //分区号
    pub desc: String,   //简介
}

pub async fn get_cookie() -> String {
    let mut ret = Vec::new();
    if let Some(cookies) = LOGININFO.await.login_info.cookie_info["cookies"].as_array() {
        for cookie in cookies {
            ret.push(format!(
                "{}={}",
                cookie["name"].as_str().unwrap_or_default(),
                cookie["value"].as_str().unwrap_or_default(),
            ))
        }
    }
    ret.join(";")
}
pub async fn upload_video(
    video_info: VideoInfo,
    filename: &String,
    multi: Option<MultiProgress>,
) -> Result<String> {
    let bilibili = LOGININFO.await.clone();

    let uploaded_videos = loop {
        if let Ok(ret) = upload(&[PathBuf::from(&filename)], 10, multi.clone()).await {
            break ret;
        }
    };
    let studio = Studio::builder()
        .desc(video_info.desc)
        .copyright(video_info.copyright)
        .source(video_info.source)
        .tag(video_info.tag)
        .tid(video_info.tid)
        .title(video_info.title)
        .videos(uploaded_videos)
        .desc_v2(None)
        .build();
    //println!("{:?}",uploaded_videos);
    let bv = loop {
        let ret = bilibili.submit_by_app(&studio, PROXY).await;
        if let Ok(result) = ret {
            if let Some(data) = result.data {
                let bv = data["bvid"].to_string();
                debug!("获得 bv = {bv}");
                break bv;
            }
        }
    };
    //println!("{:?}",ret);
    Ok(bv)
}

pub async fn append_video(
    filename: &String,
    bv: &String,
    multi: Option<MultiProgress>,
) -> Result<()> {
    let bilibili = LOGININFO.await.clone();
    let mut uploaded_videos = loop {
        if let Ok(ret) = upload(&[PathBuf::from(&filename)], 10, multi.clone()).await {
            break ret;
        }
    };
    let mut studio = bilibili
        .studio_data(&Vid::Bvid(bv.to_owned()), PROXY)
        .await?;
    studio.videos.append(&mut uploaded_videos);
    let ret = bilibili.edit_by_web(&studio).await?;
    debug!("{ret}");
    Ok(())
}

pub async fn show_video(bv: &String) -> Result<Value> {
    let bilibili = LOGININFO.await.clone();
    let video_info = bilibili
        .video_data(&Vid::Bvid(bv.to_owned()), PROXY)
        .await?;
    Ok(video_info)
}

async fn upload(
    video_path: &[PathBuf],
    limit: usize,
    multi: Option<MultiProgress>,
) -> Result<Vec<Video>> {
    let mut videos = Vec::new();
    let client = StatelessClient::default();
    let line = line::bda2(); /*match line {
                                 // Some("kodo") => line::kodo(),
                                 // Some("bda2") => line::bda2(),
                                 // Some("ws") => line::ws(),
                                 // Some("qn") => line::qn(),
                                 // Some("cos") => line::cos(),
                                 // Some("cos-internal") => line::cos_internal(),
                                 // Some(name) => panic!("不正确的线路{name}"),
                                 Some(UploadLine::Kodo) => line::kodo(),
                                 Some(UploadLine::Bda2) => line::bda2(),
                                 Some(UploadLine::Ws) => line::ws(),
                                 Some(UploadLine::Qn) => line::qn(),
                                 Some(UploadLine::Cos) => line::cos(),
                                 Some(UploadLine::CosInternal) => line::cos_internal(),
                                 None => Probe::probe().await.unwrap_or_default(),
                             };*/
    // let line = line::kodo();
    for video_path in video_path {
        //println!("{line:?}");
        let video_file = VideoFile::new(video_path)?;
        let total_size = video_file.total_size;
        let file_name = video_file.file_name.clone();
        let uploader = line
            .pre_upload(&LOGININFO.await.clone(), video_file)
            .await?;
        //Progress bar
        let pb = ProgressBar::new(total_size);
        let pb = match multi {
            Some(ref m) => m.add(pb),
            None => pb,
        };
        let name = match video_path.file_name() {
            Some(s) => s.to_str().unwrap_or(""),
            None => "",
        };
        let name = name.split("斗阵来看戏").collect::<Vec<_>>()[0];
        pb.set_style(ProgressStyle::default_bar()
            .template(format!("{{spinner:.green}} 上传{name} [{{elapsed_precise}}] [{{wide_bar:.cyan/blue}}] {{bytes}}/{{total_bytes}} ({{bytes_per_sec}}, {{eta}})").as_str())?);
        // pb.enable_steady_tick(Duration::from_secs(1));
        // pb.tick()

        let instant = Instant::now();

        //println!("{}",uploader.line.query);

        let video = uploader
            .upload(
                client.clone(),
                limit,
                |vs| {
                    vs.map(|chunk| {
                        let pb = pb.clone();
                        let chunk = chunk?;
                        let len = chunk.len();
                        Ok((Progressbar::new(chunk, pb), len))
                    })
                },
                100,
            )
            .await?;
        pb.finish_and_clear();
        let t = instant.elapsed().as_millis();
        println!(
            "Upload completed: {file_name} => cost {:.2}s, {:.2} MB/s.",
            t as f64 / 1000.,
            total_size as f64 / 1000. / t as f64
        );
        videos.push(video);
    }
    Ok(videos)
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

    pub fn progress(&mut self) -> Result<Option<Bytes>> {
        let pb = &self.pb;

        let content_bytes = &mut self.bytes;

        let n = content_bytes.remaining();

        let pc = 4096;
        if n == 0 {
            Ok(None)
        } else if n < pc {
            pb.inc(n as u64);
            Ok(Some(content_bytes.copy_to_bytes(n)))
        } else {
            pb.inc(pc as u64);

            Ok(Some(content_bytes.copy_to_bytes(pc)))
        }
    }
}

impl Stream for Progressbar {
    type Item = Result<Bytes>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match self.progress()? {
            None => Poll::Ready(None),
            Some(s) => Poll::Ready(Some(Ok(s))),
        }
    }
}
