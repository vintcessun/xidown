mod get_download_list;
mod get_video_list;
mod upload_video;
mod upload_api;
//mid:"33906231"

fn main(){
    let mid:&str="33906231";
    let mut videos = get_download_list::get_by_mid(mid).unwrap();
    //println!("Have got the videos by file");
    let urls = get_video_list::get().unwrap();
    println!("-----------------------------------------------------");
    videos = get_video_list::add_url(videos,urls);
    //println!("{:#?}",videos);
    for video in videos{
        upload_video::upload(&video).unwrap();
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