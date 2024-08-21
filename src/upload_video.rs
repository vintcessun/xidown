use crate::get_download_list::Video;
use anyhow::{anyhow, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::{info, warn};
use reqwest::header;
use reqwest::Client;
use serde_json::Value;
use std::fs;
use std::fs::remove_file;
use std::io::Write;
use std::path::Path;
use xmtv_api::VideoUrl;
use BiliupApi::{VideoInfo, _append_video, _show_video, _upload_video};

pub async fn loop_show_video(bv: &String) -> Value {
    loop {
        if let Ok(ret) = _show_video(bv).await {
            break ret;
        }
    }
}

pub async fn fliters(videos: Vec<Video>) -> Result<Vec<Video>> {
    let mut ret = Vec::with_capacity(videos.len());

    let pb = ProgressBar::new(videos.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
    .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")?);

    for video in videos {
        let ret_video = fliter(&video).await.unwrap();
        if !ret_video.range.is_empty() {
            ret.push(ret_video);
        }
        pb.inc(1);
    }

    pb.finish();

    Ok(ret)
}

async fn fliter(video: &Video) -> Result<Video> {
    info!("开始确认 video = {:?}", &video);
    if video.bv.is_empty() {
        Ok(video.clone())
    } else {
        let json = loop_show_video(&video.bv).await;
        let mut per_video: Video = Video {
            title: video.title.clone(),
            bv: video.bv.clone(),
            range: Vec::with_capacity(video.range.len()),
        };
        for i in &video.range {
            let mut exists = false;
            let videos = match json["videos"].as_array() {
                Some(ret) => ret,
                None => &vec![],
            };
            for j in videos.iter().rev() {
                if j["title"] == i.name {
                    exists = true;
                }
            }
            if !exists {
                per_video.range.push(i.to_owned())
            }
        }
        Ok(per_video)
    }
}

pub async fn upload_first(video: &Video, multi: Option<MultiProgress>) -> Option<String> {
    if video.bv.is_empty() {
        Some(loop {
            warn!("开始上传 {:?}", &video.range[0]);
            if let Ok(ret) = upload_video(&video.range[0], multi.clone()).await {
                break ret;
            }
        })
    } else {
        None
    }
}

async fn download_video(video: &VideoUrl, multi: Option<MultiProgress>) -> Result<String> {
    let filename = format!("{}.mp4", &video.name);
    let url = video.url.as_str();
    let path = Path::new(&filename);

    let client = Client::new();
    let total_size = {
        let resp = client.head(url).send().await?;
        if resp.status().is_success() {
            resp.headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|ct_len| ct_len.to_str().ok())
                .and_then(|ct_len| ct_len.parse().ok())
                .unwrap_or(0)
        } else {
            return Err(anyhow!("不能下载{} Error: {:?}", url, resp.status(),));
        }
    };

    let client = Client::new();
    let mut request = client.get(url);
    let pb = ProgressBar::new(total_size);
    let pb = match multi {
        Some(m) => m.add(pb),
        None => pb,
    };
    pb.set_style(ProgressStyle::default_bar()
    .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")?);

    if path.exists() {
        let size = path.metadata()?.len().saturating_sub(1);
        request = request.header(header::RANGE, format!("bytes={}-", size));
        pb.inc(size);
    }
    let mut source = request.send().await?;
    let mut dest = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    while let Some(chunk) = source.chunk().await? {
        dest.write_all(&chunk)?;
        pb.inc(chunk.len() as u64);
    }
    Ok(filename)
}

pub async fn upload_video(video: &VideoUrl, multi: Option<MultiProgress>) -> Result<String> {
    let videoinfo = VideoInfo {
        title: format!("{} 斗阵来看戏", &video.title),
        copyright: 2,
        source: "https://2020.xmtv.cn/search/?search_text=斗阵来看戏".to_string(),
        tag: "戏曲,斗阵来看戏".to_string(),
        tid: 180,
        desc: "自传给家里老人看方便".to_string(),
    };
    info!("任务 video = {:?}", &video);
    let filename = format!("{}.mp4", video.name);
    info!("下载到{:?}", &filename);
    download_video(video, multi.clone()).await?;
    info!("任务 video = {:?} 下载到{:?}完成", &video, &filename);
    info!("开始上传 video = {:?}", &video);
    let ret = _upload_video(videoinfo, &filename, multi).await?;
    info!("上传完成 video = {:?}", &video);
    info!("获取到bv号 ret = {:?}", &ret);
    remove_file(&filename)?;
    info!("删除文件 filename = {:?}", &filename);
    Ok(ret)
}

pub async fn append_video(
    video: &VideoUrl,
    bv: &String,
    multi: Option<MultiProgress>,
) -> Result<()> {
    info!("任务 video = {:?} 上传到 bv = {:?}", &video, &bv);
    let filename = format!("{}.mp4", video.name);
    info!("下载到{:?}", &filename);
    download_video(video, multi.clone()).await?;
    info!("任务 video = {:?} 下载到{:?}完成", &video, &filename);
    info!("开始上传 video = {:?}", &video);
    _append_video(&filename, bv, multi).await?;
    info!("上传完成 video = {:?}", &video);
    remove_file(&filename)?;
    info!("删除文件 filename = {:?}", &filename);
    Ok(())
}
