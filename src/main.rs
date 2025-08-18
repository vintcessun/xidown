mod get_download_list;
mod upload_video;
use anyhow::Result;
mod biliup_api;
use biliup_api::show_video;
use get_download_list::*;
use indicatif::MultiProgress;
use log::{debug, info};
use std::sync::Arc;
use threadpool::ThreadPool;
use upload_video::*;

#[tokio::main]
async fn main() -> Result<()> {
    let mid: &str = "33906231";
    info!("从mid:{:?}获取", &mid);
    let urls = xmtv_api::get().await?;
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();
    let videos = {
        let videos = get_by_mid(mid).await.unwrap();
        debug!("获取到videos = {videos:?}");

        debug!("获取到urls = {urls:?}");
        let videos = add_url(videos, urls);
        debug!("整理完成 videos = {videos:?}");
        fliters(videos).await.unwrap()
    };

    info!("整理完成 videos = {videos:?}");

    let m = Arc::new(MultiProgress::new());
    let pool = ThreadPool::new(4);
    for video in videos {
        let m = m.clone();
        pool.execute(move || {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    video_run(video, Some(m.as_ref().to_owned())).await;
                })
        });
    }

    pool.join();

    Ok(())
}

async fn video_run(video: Video, multi: Option<MultiProgress>) {
    let mut video = video.clone();
    'func: loop {
        let this_bv = match upload_first(&video, multi.clone()).await {
            Some(bv) => bv,
            None => video.bv.clone(),
        };

        for per in video.range[1..].iter() {
            'inner: loop {
                info!("开始上传 {:?}", per);
                if append_video(per, &video.bv, multi.clone()).await.is_ok() {
                    break 'inner;
                }
                info!("查询{}状态", &this_bv);
                let json = match show_video(&this_bv).await {
                    Ok(ret) => ret,
                    Err(_) => {
                        continue 'inner;
                    }
                };
                let state_num = json["archive"]["state"].to_string();
                info!("{}状态码为{}", &this_bv, &state_num);
                if state_num == "-2"
                    || state_num == "-3"
                    || state_num == "-4"
                    || state_num == "-5"
                    || state_num == "-12"
                    || state_num == "-16"
                    || state_num == "-100"
                {
                    video.bv.clear();
                    continue 'func;
                }
                break 'inner;
            }
        }
        break 'func;
    }
}
