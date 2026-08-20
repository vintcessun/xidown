//! 把 XMTV 的片源列表和 b 站上已有的稿件对上，算出"还差哪些分P 要传"。
//!
//! 去重一共三道闸：
//! 1. XMTV 侧按稳定 id 去重（`xmtv_api::sort_by_title`）；
//! 2. 同一个剧目名只认一个稿件——旧版是遍历所有同名稿件都往里塞，
//!    一集会被同时排进 N 个稿件；
//! 3. 本地台账 + b 站稿件里已有的分P 标题，两边都查。

use crate::bili;
use crate::config::{KEYWORD, Settings, truncate_title};
use crate::ledger::{Ledger, part_key};
use anyhow::Result;
use biliup::bilibili::Archive;
use futures::stream::{self, StreamExt};
use log::{info, warn};
use std::collections::HashMap;
use xmtv_api::VideoUrl;

/// 一部戏要做的事情。
#[derive(Debug, Clone)]
pub struct Task {
    /// 剧目名
    pub title: String,
    /// 已有稿件的 bv；空字符串表示要新投一个
    pub bv: String,
    /// 还需要上传的分P，已按时间排好序
    pub parts: Vec<VideoUrl>,
}

/// 从稿件标题反推剧目名。投稿时标题是 `{剧目名} 斗阵来看戏`。
fn title_of_archive(archive_title: &str) -> String {
    let head = match archive_title.find(KEYWORD) {
        Some(p) => &archive_title[..p],
        None => archive_title.split(' ').next().unwrap_or(archive_title),
    };
    head.trim().replace(' ', "")
}

/// 同一个剧目名可能对应多个稿件（历史上的重复投稿）。挑一个当作正主：
/// 优先选没被打回的，其次选最早创建的——后来的那些才是意外多投的。
fn pick_archive(mut candidates: Vec<Archive>) -> Archive {
    candidates.sort_by_key(|a| (a.state < 0, a.ctime));
    let chosen = candidates.remove(0);
    for dup in &candidates {
        warn!(
            "剧目 {:?} 存在重复稿件 {}（state={}），本次只往 {} 里追加",
            chosen.title, dup.bvid, dup.state, chosen.bvid
        );
    }
    chosen
}

pub async fn build_plan(settings: &Settings, ledger: &Ledger) -> Result<Vec<Task>> {
    let urls = xmtv_api::get().await?;
    info!("XMTV 片源共 {} 条", urls.len());
    let groups = xmtv_api::sort_by_title(urls);
    info!("按剧目归类后共 {} 部戏", groups.len());

    let archives = bili::list_archives().await?;
    let mut by_title: HashMap<String, Vec<Archive>> = HashMap::new();
    for a in archives {
        by_title.entry(title_of_archive(&a.title)).or_default().push(a);
    }
    let chosen: HashMap<String, Archive> = by_title
        .into_iter()
        .map(|(t, v)| (t, pick_archive(v)))
        .collect();

    // 先做本地筛选，再决定要不要为这部戏去查一次 b 站接口
    let mut candidates: Vec<(String, String, Vec<VideoUrl>)> = Vec::new();
    for group in groups {
        if let Some(f) = &settings.only_title
            && !group.title.contains(f.as_str())
        {
            continue;
        }
        let bv = chosen
            .get(&group.title)
            .map(|a| a.bvid.clone())
            .unwrap_or_default();

        let mut parts = Vec::new();
        for part in group.range {
            if ledger.contains(&part_key(&part), Some(&bv)).await.is_some() {
                continue;
            }
            parts.push(part);
        }
        if !parts.is_empty() {
            candidates.push((group.title, bv, parts));
        }
    }
    info!("台账过滤后还有 {} 部戏需要检查", candidates.len());

    // 只对"已有稿件"的部分去查 b 站上已有的分P，并发查，别一部一部排队
    let tasks: Vec<Task> = stream::iter(candidates)
        .map(|(title, bv, parts)| async move {
            if bv.is_empty() {
                return Task { title, bv, parts };
            }
            let existing = match bili::part_titles(&bv).await {
                Ok(t) => t,
                Err(e) => {
                    // 查不到就当作没有已存在的分P，后面追加前还会再查一次兜底
                    warn!("查询 {bv} 的分P 列表失败({e})，按未知处理");
                    Vec::new()
                }
            };
            let parts = parts
                .into_iter()
                .filter(|p| !existing.contains(&truncate_title(&p.name)))
                .collect();
            Task { title, bv, parts }
        })
        .buffer_unordered(8)
        .filter(|t| {
            let empty = t.parts.is_empty();
            async move { !empty }
        })
        .collect()
        .await;

    let mut tasks = tasks;
    // 分P 少的排前面，先把零碎的收掉，进度看起来也更顺
    tasks.sort_by(|a, b| {
        a.parts
            .len()
            .cmp(&b.parts.len())
            .then_with(|| a.title.cmp(&b.title))
    });

    if let Some(n) = settings.max_parts {
        for t in &mut tasks {
            t.parts.truncate(n.max(1));
        }
    }
    if let Some(n) = settings.max_archives {
        tasks.truncate(n);
    }

    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_title_of_archive() {
        assert_eq!(title_of_archive("白蛇传 斗阵来看戏"), "白蛇传");
        assert_eq!(title_of_archive("陈三五娘 斗阵来看戏 20240101"), "陈三五娘");
        assert_eq!(title_of_archive("没有关键字的标题"), "没有关键字的标题");
    }

    fn archive(bvid: &str, state: i16, ctime: u64) -> Archive {
        Archive {
            bvid: bvid.into(),
            title: "甲 斗阵来看戏".into(),
            state,
            ctime,
            ..Default::default()
        }
    }

    #[test]
    fn test_pick_archive_prefers_alive_then_oldest() {
        let got = pick_archive(vec![
            archive("BV_new", 0, 200),
            archive("BV_old", 0, 100),
            archive("BV_dead", -2, 50),
        ]);
        assert_eq!(got.bvid, "BV_old");

        // 全都被打回时退回最早的那个
        let got = pick_archive(vec![archive("BV_b", -2, 200), archive("BV_a", -4, 100)]);
        assert_eq!(got.bvid, "BV_a");
    }
}
