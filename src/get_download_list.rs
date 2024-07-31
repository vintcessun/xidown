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
            //.header("cookie","CURRENT_BLACKGAP=0;CURRENT_FNVAL=4048;CURRENT_QUALITY=112;DedeUserID=33906231;DedeUserID__ckMd5=779e057704a961f6;FEED_LIVE_VERSION=V8;PVID=1;SESSDATA=4304a554%2C1733125187%2C47b9a%2A61CjCEaV64s3It3tBx0gRCoCpSW4-YERrsqvQWq3umBv1weQfNXaXk6BelYv0A5ialzIUSVjg4dDF5VFBRTE13a0NhMXJhNUpNdUkyQzRHR2FFa2ZRZGVDRVNobTFTZmpZdXNiVHdyY0JaZUg4RmpDSkYtLTY5Rk14WU5EQXNUWlYtVjhfdWJ1dzVnIIEC;_uuid=2D14F4CA-7F4C-CFA10-FCAA-7FC8BA61963E36151infoc;b_lsid=910E610D2E_18FE755D94C;b_nut=1689656934;bili_jct=3f149a74eabfe364ee9bb2c4b28c7809;bili_ticket=eyJhbGciOiJIUzI1NiIsImtpZCI6InMwMyIsInR5cCI6IkpXVCJ9.eyJleHAiOjE3MTc4MzI0NzUsImlhdCI6MTcxNzU3MzIxNSwicGx0IjotMX0.iF9V5oX9CzhSlQCeT54w03LsPLV80JUmeCD249iu5_Q;bili_ticket_expires=1717832415;browser_resolution=1482-708;buvid3=15B751A7-0701-4CB1-6E70-9D9098E8D18834638infoc;buvid4=9C5AAA34-984E-4552-2A59-BE365B73093936163-023071813-2ehoPqLPBEbTk16Vhj%2BUZQ%3D%3D;buvid_fp=df891ad2cbbac3f9b5ea0ca0445bbc64;dy_spec_agreed=1;fingerprint=04c5a12b8b76ab29996f980421b35264;hit-dyn-v2=1;home_feed_column=5")
            .header("Referer","https://space.bilibili.com/33906231/video")
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
