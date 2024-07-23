mod get_download_list;
mod get_video_list;
mod upload_video;
//mid:"33906231"

fn main(){
    //let mid:&str="33906231";
    let mut videos = get_download_list::get_by_file("download_list.txt").unwrap();
    //println!("Have got the videos by file");
    let urls = get_video_list::get().unwrap();
    println!("-----------------------------------------------------");
    videos = get_video_list::add_url(videos,urls);
    //println!("{:#?}",videos);
    for video in videos{
        upload_video::upload(&video);
    }
}