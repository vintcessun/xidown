use anyhow::Result;
use log::{debug, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use xmtv_api::VideoUrl;

use crate::biliup_api::get_cookie;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Video {
    pub title: String,
    pub bv: String,
    pub range: Vec<VideoUrl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoDisplay {
    origin: Option<Value>,
    usr_action_txt: Option<String>,
    relation: Option<Value>,
    live_info: Option<Value>,
    emoji_info: Option<Value>,
    highlight: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoDescription {
    uid: u64,
    r#type: u8,
    rid: u128,
    acl: i64,
    view: u64,
    repost: u64,
    comment: u64,
    like: u64,
    is_liked: u64,
    dynamic_id: u128,
    timestamp: u64,
    pre_dy_id: u64,
    orig_dy_id: u64,
    orig_type: u64,
    user_profile: Value,
    spec_type: u64,
    uid_type: u64,
    stype: u64,
    r_type: u64,
    inner_id: u64,
    status: u64,
    dynamic_id_str: String,
    pre_dy_id_str: String,
    orig_dy_id_str: String,
    rid_str: String,
    origin: Option<Value>,
    bvid: String,
    previous: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoCard {
    desc: VideoDescription,
    card: String,
    extend_json: String,
    display: VideoDisplay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchData {
    total: usize,
    cards: Vec<VideoCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchBody {
    code: u8, //code非0就是出问题了，可能是请求速度太快了被b站过滤了
    message: String,
    data: SearchData,
    ttl: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoCardInner {
    aid: u128,
    cid: u64,
    ctime: u64,
    desc: String,
    dimension: Value,
    duration: u64,
    dynamic: String,
    first_frame: String,
    jump_url: String,
    owner: Value,
    pic: String,
    pubdate: u64,
    short_link_v2: String,
    stat: Value,
    state: u64,
    tid: u64,
    title: String,
    tname: String,
    videos: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicCardInner {
    user: Value,
    item: Value,
    origin: Value,
    origin_extend_json: Value,
    origin_user: Value,
}

pub async fn get_by_mid(mid: &str) -> Result<Vec<Video>> {
    let mut i = 1;
    let mut ret = Vec::new();
    'retry: loop {
        let page_url = format!("https://api.bilibili.com/x/space/dynamic/search?keyword=斗阵来看戏&pn={i}&ps=30&mid={mid}");
        info!("获取已上传视频 i = {i:?}, page_url = {page_url:?}");
        let res = match Client::new()
            .get(page_url)
            .header("User-Agent","Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36")
            .header("Cookie",get_cookie().await)
            .send()
            .await{
                Ok(e)=>e,
                Err(e)=>{
                    warn!("错误 {e} 重试...第{i}页");
                    continue 'retry;
                }
            };
        debug!("请求结果 res = {res:?}",);

        let data = res.json::<SearchBody>().await?;
        debug!("data = {data:?}");
        if data.code == 0 {
            let cards = data.data.cards;
            if cards.is_empty() {
                break 'retry;
            }
            for ele in cards {
                let card = ele.card;
                debug!("card = {card}");
                let per_card = match serde_json::from_str::<VideoCardInner>(&card) {
                    Ok(e) => e,
                    Err(e) => {
                        let dynamic_card = serde_json::from_str::<DynamicCardInner>(&card)
                            .unwrap_or_else(move |e| {
                                warn!("解析card错误 第{i}个 {e} card = {card}");
                                panic!()
                            });
                        warn!("解析到动态 {dynamic_card:?} ...跳过 {e}");
                        i += 1;
                        continue 'retry;
                    }
                };
                let title = per_card.title;
                let title = title.split(' ').collect::<Vec<_>>()[0].to_string();
                let bv = ele.desc.bvid;
                let video: Video = Video {
                    title,
                    bv,
                    range: Vec::new(),
                };
                debug!("获取到 video = {video:?}");
                ret.push(video);
            }
            i += 1;
        } else {
            continue 'retry;
        }
    }
    info!("获取完成 ret = {ret:?}");
    Ok(ret)
}

pub fn add_url(mut videos: Vec<Video>, urls: Vec<VideoUrl>) -> Vec<Video> {
    for url in &urls {
        let mut exists = false;
        for video in &mut videos {
            if url.title == video.title {
                exists = true;
                video.range.push(url.clone());
            }
        }
        if !exists {
            let mut video = Video {
                title: url.title.clone(),
                bv: "".to_string(),
                range: Vec::new(),
            };
            video.range.push(url.clone());
            videos.push(video);
        }
    }
    for video in &mut videos {
        video.range.sort_by(|a, b| a.time.cmp(&b.time));
    }
    videos
}

#[cfg(test)]
mod test {
    use crate::biliup_api::get_cookie;

    use super::*;

    #[tokio::test]
    async fn test_get_info() {
        env_logger::Builder::new()
            .filter_level(log::LevelFilter::Info)
            .init();
        println!("{:?}", get_by_mid("33906231").await.unwrap());
    }

    #[tokio::test]
    async fn test_login_info() {
        println!("{}", get_cookie().await);
    }
}
