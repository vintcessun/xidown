use anyhow::{Context, Result};
use biliup::client::{Client, LoginInfo};
use biliup::video::{BiliBili, Video, Vid};
use biliup::{line, VideoFile};
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::error::Error;
use indicatif::{ProgressBar, ProgressStyle};
use futures::{Stream, StreamExt};
use bytes::{Buf, Bytes};
use std::pin::Pin;
use std::task::Poll;
use reqwest::Body;
use serde_json::Value;
use std::io::Seek;


pub fn upload_video(title:&String,filename:&String)->Result<String,Box<dyn Error>>{
    tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(async {
        _upload_video(title,filename).await
    })
}

pub fn append_video(filename:&String,bv:&String)->Result<(),Box<dyn Error>>{
    tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(async {
        _append_video(filename,bv).await
    })
}

pub fn show_video(bv:&String)->Result<Value,Box<dyn Error>>{
    tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(async {
        _show_video(bv).await
    })
}

async fn _upload_video(title:&String,filename:&String)->Result<String,Box<dyn Error>>{
    let cookie_file=PathBuf::from("cookies.json");

    let client = Client::new();
    let f = fopen_rw(&cookie_file)?;
    let login_info = match client.login_by_cookies(f).await{
        Ok(ret)=>{ret}
        Err(_)=>{
            renew(Client::new(),cookie_file.clone()).await?;
            let f = fopen_rw(&cookie_file)?;
            client.login_by_cookies(f).await?
        }
    };

    let uploaded_videos = loop{
        match upload(&[PathBuf::from(&filename)], &client, 10).await{
            Ok(ret)=>{
                break ret;
            },
            Err(_)=>{}
        }
    };
    let mut builder = biliup::video::Studio::builder()
    .desc("自传给家里老人看方便".to_string())
    .copyright(2)
    .source("https://2020.xmtv.cn/search/?search_text=斗阵来看戏".to_string())
    .tag("戏曲,斗阵来看戏".to_string())
    .tid(180)
    .title(format!("{} 斗阵来看戏",title))
    .videos(uploaded_videos)
    .build();
    //println!("{:?}",uploaded_videos);
    let bv = loop{
        let ret = &builder.submit(&login_info).await;
        match ret{
            Ok(result)=>{
                let bv=result["data"]["bvid"].to_string();
                break bv;
            }
            Err(_)=>{}
        }
    };
    //println!("{:?}",ret);
    Ok(bv)
}

async fn _append_video(filename:&String,bv:&String)->Result<(),Box<dyn Error>>{
    let cookie_file=PathBuf::from("cookies.json");

    let client = Client::new();
    let login_info = client.login_by_cookies(fopen_rw(cookie_file)?).await?;
    let mut uploaded_videos = loop{
        match upload(&[PathBuf::from(&filename)], &client, 10).await{
            Ok(ret)=>{
                break ret;
            },
            Err(_)=>{}
        }
    };
    let mut studio = BiliBili::new(&login_info, &client).studio_data(Vid::Bvid(bv.clone())).await?;
    studio.videos.append(&mut uploaded_videos);
    let _ret = studio.edit(&login_info).await?;
    //println!("{}",_ret);
    Ok(())
}

async fn _show_video(bv:&String)->Result<Value,Box<dyn Error>>{
    let cookie_file=PathBuf::from("cookies.json");

    let client = Client::new();
    let login_info = client.login_by_cookies(fopen_rw(cookie_file)?).await?;
    let video_info = BiliBili::new(&login_info, &client).video_data(Vid::Bvid(bv.clone())).await?;
    Ok(video_info)
}

async fn renew(client: Client, user_cookie: PathBuf) -> Result<()> {
    let mut file = fopen_rw(user_cookie)?;
    let login_info: LoginInfo = serde_json::from_reader(&file)?;
    let new_info = client.renew_tokens(login_info).await?;
    file.rewind()?;
    file.set_len(0)?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(&file), &new_info)?;
    println!("{new_info:?}");
    Ok(())
}

pub async fn upload(
    video_path: &[PathBuf],
    client: &Client,
    limit: usize,
) -> Result<Vec<Video>> {
    let mut videos = Vec::new();
    let line = line::bda2();/*match line {
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
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")?);
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