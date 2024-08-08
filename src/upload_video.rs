use crate::get_download_list::Video;
use crate::get_video_list::{get_video_url,VideoUrl};
use crate::upload_api;
use std::fs::File;
use std::io::Write;
use std::error::Error;
use std::fs::remove_file;
use log::{info, warn, error};
use curl::easy::Easy;

fn fliter(video:&Video)->Result<Video,Box<dyn Error>>{
    info!("开始确认 video = {:?}",&video);
    if video.bv.is_empty(){
        //warn!("应该上传video下的所有：{:?}",video);
        Ok(video.clone())
        //warn!("开始上传 {:?}",&video.range[0]);
        //let bv = upload_video(&video.range[0])?;
        //for i in 1..video.range.len(){
        //    warn!("开始上传 {:?} 到 bv = {:?}",&video.range[i],&bv);
        //    append_video(&video.range[i],&bv)?;
        //}
    }
    else{
        let json = upload_api::show_video(&video.bv)?;
        let mut per_video:Video = Video{title:video.title.clone(),bv:video.bv.clone(),range:vec![]};
        for i in &video.range{
            //println!("[{}]{}",&video.title,&video.bv);
            //println!("[{}]{}",&video.title,&text);
            let mut exists = false;
            let videos = match json["videos"].as_array(){
                Some(ret)=>{ret}
                None=>{&vec![]}
            };
            for j in videos.iter().rev(){
                if j["title"]==i.name{
                    exists=true;
                }
            }
            if !exists{
                //warn!("应该上传{:?} 到 bv = {:?}",&i,&video.bv);
                per_video.range.push(i.clone())
                //append_video(&i,&video.bv)?;
            }
        }
        Ok(per_video)
    }
}

pub fn fliters(videos:Vec<Video>)->Result<Vec<Video>,Box<dyn Error>>{
    info!("开始确认");
    let mut ret = vec![];
    for i in videos{
        let one = fliter(&i)?;
        if !one.range.is_empty(){
            ret.push(one);
        }
    }
    Ok(ret)
}

pub fn upload_range_video(video:Video)->Result<(),Box<dyn Error>>{
    if video.bv.is_empty(){
        warn!("开始上传 {:?}",&video.range[0]);
        let bv = upload_video(&video.range[0])?;
        for i in 1..video.range.len(){
            warn!("开始上传 {:?} 到 bv = {:?}",&video.range[i],&bv);
            append_video(&video.range[i],&bv)?;
        }
    }
    else{
        for i in video.range{
            warn!("应该上传{:?} 到 bv = {:?}",&i,&video.bv);
            append_video(&i,&video.bv)?;
        }
    }
    Ok(())
}

fn download_video(video:&VideoUrl,filename:&String)->Result<(),Box<dyn Error>>{
    let url = get_video_url(&video.url)?;
    let mut curl = Easy::new();
    
    info!("\"{}\"=>\"{}\"",&url,&filename);
    curl.url(&url)?;
    curl.progress(true)?;
    curl.progress_function(|total_download_bytes,cur_download_bytes,_total_upload_bytes,_cur_upload_bytes|{
        if total_download_bytes>0.0{
            left_print(format!("已下载:{:.2}%",cur_download_bytes/total_download_bytes*100.0).as_str());
        }
        else{
            left_print("已下载:0%");
        }
        true
    })?;
    loop{
        let mut file = File::create(filename)?;
        curl.write_function(move |data| {
            file.write_all(data).unwrap();
            Ok(data.len())
        })?;
        match curl.perform(){
            Ok(_)=>{
                println!();
                break;
            }
            Err(_)=>{
                error!("下载失败，正在重试")
            }
        }
    };

    Ok(())
}

fn left_print(msg:&str){
    let mut str:String = msg.to_string();
    for _ in 0..str.len(){
        str.push('\x08')
    }
    print!("{}",str);
}

/*
fn left_print(msg:&str){
    let mut str:String = msg.to_string();
    for _ in 0..str.len(){
        str.push('\x08')
    }
    print!("{}",str);
}
*/

fn upload_video(video:&VideoUrl)->Result<String,Box<dyn Error>>{
    info!("任务 video = {:?}",&video);
    let filename = format!("{}.mp4",video.name);
    info!("下载到{:?}",&filename);
    download_video(video,&filename)?;
    info!("任务 video = {:?} 下载到{:?}完成",&video,&filename);
    info!("开始上传 video = {:?}",&video);
    let ret = upload_api::upload_video(&video.title,&filename)?;
    info!("上传完成 video = {:?}",&video);
    info!("获取到bv号 ret = {:?}",&ret);
    remove_file(&filename)?;
    info!("删除文件 filename = {:?}",&filename);
    Ok(ret)
}

fn append_video(video:&VideoUrl,bv:&String)->Result<(),Box<dyn Error>>{
    info!("任务 video = {:?} 上传到 bv = {:?}",&video,&bv);
    let filename = format!("{}.mp4",video.name);
    info!("下载到{:?}",&filename);
    download_video(video,&filename)?;
    info!("任务 video = {:?} 下载到{:?}完成",&video,&filename);
    info!("开始上传 video = {:?}",&video);
    upload_api::append_video(&filename,bv)?;
    info!("上传完成 video = {:?}",&video);
    remove_file(&filename)?;
    info!("删除文件 filename = {:?}",&filename);
    Ok(())
}