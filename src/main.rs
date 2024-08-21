mod get_download_list;
mod upload_video;
use indicatif::MultiProgress;
//use chrono::Utc;
use anyhow::Result;
use get_download_list::*;
use log::info;
use std::env::set_var;
use std::sync::Arc;
use threadpool::ThreadPool;
use upload_video::*;

const ERROR_LIST: [&str; 2] = ["-2", "-4"];
static mut UPLOAD_VIDEO: Vec<String> = Vec::new();

#[tokio::main]
async fn main() -> Result<()> {
    let mid: &str = "33906231";
    info!("从mid:{:?}获取", &mid);
    let videos = get_by_mid(mid).await?;

    info!("获取到videos = {:?}", &videos);
    let urls = xmtv_api::get()?;
    info!("获取到urls = {:?}", urls);
    let videos = add_url(videos, urls);
    info!("整理完成 videos = {:?}", &videos);
    let videos = fliters(videos).await?;

    set_var("RUST_LOG", "info");
    env_logger::init();
    let m = Arc::new(MultiProgress::new());
    let pool = ThreadPool::new(3);
    for video in videos {
        let m = m.clone();
        pool.execute(move || {
            futures::executor::block_on(async {
                video_run(video, Some(m.as_ref().to_owned())).await;
            })
        });
    }

    pool.join();

    Ok(())
}

async fn video_run(video: Video, multi: Option<MultiProgress>) {
    let mut video = video;
    let mut this_bv = match upload_first(&video, multi.clone()).await {
        Some(bv) => bv,
        None => video.bv.clone(),
    };
    unsafe {
        UPLOAD_VIDEO.push(this_bv.clone());
    };

    'outer: loop {
        for per in video.range[1..].iter() {
            loop {
                info!("查询{}状态", &this_bv);
                let json = loop_show_video(&this_bv).await;
                let state_num = json["archive"]["state"].to_string();
                info!("{}状态码为{}", &this_bv, &state_num);
                if ERROR_LIST.contains(&state_num.as_str()) {
                    video.bv = "".to_string();
                    this_bv = upload_first(&video, multi.clone()).await.unwrap();
                    continue 'outer;
                }
                if append_video(per, &video.bv, multi.clone()).await.is_ok() {
                    break;
                }
            }
        }

        break;
    }

    unsafe {
        let uploaded = UPLOAD_VIDEO.clone();
        let mut ret = Vec::with_capacity(UPLOAD_VIDEO.len());
        for bv in uploaded {
            if bv != this_bv {
                ret.push(bv);
            }
        }
        UPLOAD_VIDEO = ret;
    }
}

/*
fn main(){
    //let title = "test2".to_string();
    let filename = "兄弟双状元（3） 斗阵来看戏 2024.07.19 - 厦门卫视.mp4".to_string();
    let bv = "BV1sz421i7Tf".to_string();
    let ret=upload_api::show_video(&bv).unwrap();
    println!("{}",ret["videos"][0]["title"].to_string());
    //println!("{}",bv);
}
*/
