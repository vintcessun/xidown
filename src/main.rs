mod get_download_list;
mod get_video_list;
mod upload_api;
mod upload_video;
use chrono::Utc;
use log::info;
use std::env::{args, set_var};
use std::process::Command;
use threadpool::ThreadPool;

const ERROR_LIST: [&str; 2] = ["-2", "-4"];

fn main() {
    set_var("RUST_LOG", "info");
    env_logger::init();

    let mid: &str = "33906231";
    info!("从mid:{:?}获取", &mid);
    let mut videos = get_download_list::get_by_mid(mid).unwrap();
    info!("获取到videos = {:?}", &videos);
    let urls = get_video_list::get().unwrap();
    info!("获取到urls = {:?}", urls);
    videos = get_video_list::add_url(videos, urls);
    info!("整理完成 videos = {:?}", &videos);
    videos = upload_video::fliters(videos).unwrap();
    info!("筛选完成 videos = {:?}", &videos);

    let mut bvs = Vec::with_capacity(videos.len());

    let pool = ThreadPool::new(3);
    for video in videos {
        bvs.push(video.bv.clone());
        pool.execute(move || {
            upload_video::upload_range_video(video).unwrap();
        });
    }
    let mut last_time = Utc::now().time();
    loop {
        if pool.queued_count() == 0 {
            break;
        }

        let now_time = Utc::now().time();
        let delta = (now_time - last_time).num_seconds() as usize;
        if delta > 3600 {
            last_time = now_time;
            if !each_bv_state(&bvs) {
                let args = args().collect::<String>();
                Command::new(format!("start {}", args));
                break;
            }
        }
    }
}

fn each_bv_state(bvs: &[String]) -> bool {
    'outer: loop {
        let mut state = true;
        for bv in bvs {
            let json = match upload_api::show_video(bv) {
                Ok(ret) => ret,
                Err(_) => continue 'outer,
            };
            let state_num = json["archive"]["state"].to_string();
            state = state && !ERROR_LIST.contains(&state_num.as_str());
        }
        break state;
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
