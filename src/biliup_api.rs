use anyhow::{anyhow, Context, Result};
use async_static::async_static;
use biliup::client::{Client, LoginInfo};
use biliup::video::{BiliBili, Vid, Video};
use biliup::{line, VideoFile};
use bytes::{Buf, Bytes};
use futures::{Stream, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use lazy_static::lazy_static;
use log::error;
use reqwest::Body;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::Instant;

const COOKIE_FILE: &str = r#"C:\Users\vintc\Desktop\rust\xidown\cookies.json"#;
lazy_static! {
    static ref CLIENT: Arc<Client> = Arc::new(Client::new());
}
async_static! {
    static ref LOGININFO: Arc<LoginInfo> = Arc::new(CLIENT.login_by_cookies({
        fopen_rw(COOKIE_FILE).unwrap()
    }).await.unwrap());
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
    if let Some(cookies) = LOGININFO.await.cookie_info["cookies"].as_array() {
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
    let client = CLIENT.as_ref();
    let login_info = LOGININFO.await.as_ref();

    let uploaded_videos = loop {
        if let Ok(ret) = upload(&[PathBuf::from(&filename)], client, 10, multi.clone()).await {
            break ret;
        }
    };
    let mut builder = biliup::video::Studio::builder()
        .desc(video_info.desc)
        .copyright(video_info.copyright)
        .source(video_info.source)
        .tag(video_info.tag)
        .tid(video_info.tid)
        .title(video_info.title)
        .videos(uploaded_videos)
        .build();
    //println!("{:?}",uploaded_videos);
    let bv = loop {
        let ret = &builder.submit(login_info).await;
        if let Ok(result) = ret {
            let bv = result["data"]["bvid"].to_string();
            break bv;
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
    let client = CLIENT.as_ref();
    let login_info = LOGININFO.await.as_ref();
    let mut uploaded_videos = loop {
        if let Ok(ret) = upload(&[PathBuf::from(&filename)], client, 10, multi.clone()).await {
            break ret;
        }
    };
    let mut studio = BiliBili::new(login_info, client)
        .studio_data(Vid::Bvid(bv.to_owned()))
        .await?;
    studio.videos.append(&mut uploaded_videos);
    let _ret = studio.edit(login_info).await?;
    //println!("{}",_ret);
    Ok(())
}

pub async fn show_video(bv: &String) -> Result<Value> {
    let client = Client::new();
    let login_info = LOGININFO.await.as_ref();
    let video_info = match BiliBili::new(login_info, &client)
        .video_data(Vid::Bvid(bv.to_owned()))
        .await
    {
        Ok(v) => v,
        Err(e) => {
            error!("{}", e);
            return Err(anyhow!("Errors {}", e));
        }
    };
    Ok(video_info)
}

async fn upload(
    video_path: &[PathBuf],
    client: &Client,
    limit: usize,
    multi: Option<MultiProgress>,
) -> Result<Vec<Video>> {
    let mut videos = Vec::new();
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
        let uploader = line.to_uploader(video_file);
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
            .upload(client, limit, |vs| {
                vs.map(|chunk| {
                    let pb = pb.clone();
                    let chunk = chunk?;
                    let len = chunk.len();
                    Ok((Progressbar::new(chunk, pb), len))
                })
            })
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

impl From<Progressbar> for Body {
    fn from(async_stream: Progressbar) -> Self {
        Body::wrap_stream(async_stream)
    }
}

#[inline]
fn fopen_rw<P: AsRef<Path>>(path: P) -> Result<std::fs::File> {
    let path = path.as_ref();
    std::fs::File::options()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| String::from("open cookies file: ") + &path.to_string_lossy())
}
