use crate::get_download_list::Video;
use crate::get_video_list::{self, VideoUrl};
use crate::upload_api;
use curl::easy::Easy;
use std::error::Error;
use std::fs::{File,remove_file};
use std::io::Write;


pub fn upload(video:&Video)->Result<(),Box<dyn Error>>{
    if video.bv==""{
        println!("应该上传video下的所有：{:?}",video);
        let bv = upload_video(&video.range[0])?;
        for i in 1..video.range.len(){
            append_video(&video.range[i],&bv)?;
        }
    }
    else{
        let json = upload_api::show_video(&video.bv)?;
        for i in &video.range{
            //println!("[{}]{}",&video.title,&video.bv);
            //println!("[{}]{}",&video.title,&text);
            let mut exists = false;
            for j in json["videos"].as_array().unwrap().into_iter().rev(){
                if j["title"]==i.name{
                    exists=true;
                }
            }
            if !exists{
                println!("应该上传{:?}",&i);
                append_video(&i,&video.bv)?;
            }
        }
    }
    Ok(())
}

fn download_video(video:&VideoUrl,filename:&String)->Result<(),Box<dyn Error>>{
    let url = get_video_list::get_video_url(&video.url)?;
    let mut curl = Easy::new();
    
    println!("\"{}\"=>\"{}\"",&url,&filename);
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
        let mut file = File::create(&filename)?;
        curl.write_function(move |data| {
            file.write(&data).unwrap();
            Ok(data.len())
        })?;
        match curl.perform(){
            Ok(_)=>{
                println!("");
                break;
            }
            Err(_)=>{}
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

fn upload_video(video:&VideoUrl)->Result<String,Box<dyn Error>>{
    let filename = format!("{}.mp4",video.name);
    download_video(&video,&filename)?;
    let ret = upload_api::upload_video(&video.title,&filename)?;
    remove_file(&filename)?;
    Ok(ret)
}

fn append_video(video:&VideoUrl,bv:&String)->Result<(),Box<dyn Error>>{
    let filename = format!("{}.mp4",video.name);
    download_video(&video,&filename)?;
    let ret = upload_api::append_video(&filename,&bv)?;
    remove_file(&filename)?;
    Ok(ret)
}