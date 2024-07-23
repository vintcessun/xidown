use url::Url;
use reqwest::blocking::Client;
use std::error::Error;
use serde_json::Value;

use crate::get_download_list::Video;

#[derive(Debug)]
#[derive(Clone)]
pub struct VideoUrl{
    pub title:String,
    pub name:String,
    pub url:String,
    pub time:u32
}

pub fn get()->Result<Vec<VideoUrl>,Box<dyn Error>>{
    println!("开始下载");
    let url = Url::parse("https://mapi1.kxm.xmtv.cn/api/open/xiamen/web_search_list.php?count=10&search_text=%E6%96%97%E9%98%B5%E6%9D%A5%E7%9C%8B%E6%88%8F&offset=0&bundle_id=livmedia&order_by=publish_time&time=0&with_count=1")?;
    let res = Client::new().get(url).send()?;
    let text:String = res.text()?;
    let json:Value = serde_json::from_str(text.as_str())?;
    let mut ret:Vec<VideoUrl> = vec![];
    let data = json["data"].as_array().unwrap().into_iter().rev();
    for i in data{
        let name = i["title"].to_string().replace("\"","");
        let title = name[0..name.find("斗阵来看戏").unwrap()].replace("（","(").split("(").collect::<Vec<_>>()[0].replace(" ","");
        let url_into_share = Url::parse(i["content_urls"]["share"].as_str().unwrap())?;
        let res = Client::new().get(url_into_share).send()?;
        let text = res.text()?;
        let text = text[(text.find("<source src=").unwrap()+13)..].to_string();
        let download_url = text[..(text.find("\"").unwrap())].to_string();
        let t = &name[(name.find("斗阵来看戏").unwrap()+"斗阵来看戏".len())..];
        let t = t.split(" ").collect::<Vec<_>>()[1].replace(".","");
        let t = t.parse::<u32>()?;
        let video = VideoUrl{title:title,name:name,url:download_url,time:t};
        println!("{:?}",video);
        ret.push(video);
    }
    return Ok(ret);
}

pub fn add_url(mut videos:Vec<Video>,urls:Vec<VideoUrl>)->Vec<Video>{
    for url in &urls{
        let mut exists=false;
        for video in &mut videos{
            if url.title==video.title{
                exists=true;
                video.range.push(url.clone());
            }
        }
        if exists==false{
            let mut video=Video{title:url.title.clone(),bv:"".to_string(),range:vec![]};
            video.range.push(url.clone());
            videos.push(video);
        }
    }
    for video in &mut videos{
        video.range.sort_by(|a,b| a.time.cmp(&b.time));
    }
    return videos;
}