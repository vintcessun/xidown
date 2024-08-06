mod get_download_list;
mod get_video_list;
mod upload_video;
mod upload_api;
use log::info;
use std::env::set_var;
use threadpool::ThreadPool;

fn main(){
    set_var("RUST_LOG","info");
    env_logger::init();

    let mid:&str="33906231";
    info!("从mid:{:?}获取",&mid);
    let mut videos = get_download_list::get_by_mid(mid).unwrap();
    info!("获取到videos = {:?}",&videos);
    let urls = get_video_list::get().unwrap();
    info!("获取到urls = {:?}",urls);
    videos = get_video_list::add_url(videos,urls);
    info!("整理完成 videos = {:?}",&videos);
    videos = upload_video::fliters(videos).unwrap();
    info!("筛选完成 videos = {:?}",&videos);

    let pool = ThreadPool::new(2);
    for video in videos{
        pool.execute(move||{
            upload_video::upload_range_video(video).unwrap();
        });
    }
    pool.join();
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