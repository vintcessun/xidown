use url::Url;
use reqwest::blocking::Client;
use std::error::Error;
use serde_json::Value;
use std::fs::File;
use std::io::Write;
use std::fs::read_to_string;
use crate::get_video_list::VideoUrl;

#[derive(Debug)]
pub struct Video{
    pub title:String,
    pub bv:String,
    pub range:Vec<VideoUrl>
}

pub fn save(filename:&str,video:&Vec<Video>)->Result<(),Box<dyn Error>>{
    let mut file = File::create(filename)?;
    for i in video{
        write!(file,"{} {}\n",i.title,i.bv)?;
    }
    Ok(())
}

pub fn get_by_mid(mid:&str)->Result<Vec<Video>,Box<dyn Error>>{
    let mut i = 1;
    let mut ret:Vec<Video> = vec![];
    loop{
        let page_url = Url::parse(format!("https://api.bilibili.com/x/space/dynamic/search?keyword=%E6%96%97%E9%98%B5%E6%9D%A5%E7%9C%8B%E6%88%8F&pn={}&ps=30&mid={}",i,mid).as_str())?;
        let res = Client::new()
            .get(page_url)
            .header("User-Agent","Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/108.0.0.0 Safari/537.36")
            .send()?;
        let text: String = res.text()?;
        let json:Value = serde_json::from_str(&text)?;
        if json["code"]==0{
            if json["data"]["cards"].as_array().unwrap().len() == 0{
                break;
            }
            for i in json["data"]["cards"].as_array().unwrap(){
                let per_card:Value = serde_json::from_str(&i["card"].as_str().unwrap())?;
                let title: String = per_card["title"].as_str().unwrap()
                                .split(" ").collect::<Vec<_>>()[0].to_string();
                let bv: String = i["desc"]["bvid"].as_str().unwrap().to_string();
                let video: Video = Video{title:title,bv:bv,range:vec![]};
                //println!("{:#?}",video);
                ret.push(video);
            }
        }
        i+=1;
    }
    Ok(ret)
}


pub fn get_by_file(filename:&str)->Result<Vec<Video>,Box<dyn Error>>{
    let mut ret:Vec<Video>=vec![];
    for line in read_to_string(filename)?.lines(){
        let i=line.split(" ").collect::<Vec<_>>();
        let (title,bv) = (i[0].to_string(),i[1].to_string());
        let video=Video{title:title,bv:bv,range:vec![]};
        ret.push(video);
    }
    Ok(ret)
}
