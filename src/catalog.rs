//! 把 XMTV 的片源列表和 b 站上已有的稿件对上，算出"还差哪些分P 要传"。
//!
//! 去重一共三道闸：
//! 1. XMTV 侧按稳定 id 去重（`xmtv_api::sort_by_title`）；
//! 2. 同一个剧目名只认一个稿件——旧版是遍历所有同名稿件都往里塞，
//!    一集会被同时排进 N 个稿件；
//! 3. 本地台账 + b 站稿件里已有的分P 标题，两边都查。

use crate::bili;
use crate::config::{CATALOG_CACHE, KEYWORD, Settings, truncate_title};
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
/// 优先选没被锁定/打回的，其次选最早创建的——后来的那些才是意外多投的。
///
/// 注意判定"废了"要用 `state_is_dead` 而不是 `state < 0`：
/// 待审(-1)、审核中(-30)、定时发布(-40) 都是正常的中间状态，仍然可以追加分P，
/// 不该被当成和"被锁定"一样的坏稿件。
fn pick_archive(mut candidates: Vec<Archive>) -> Archive {
    candidates.sort_by_key(|a| (bili::state_is_dead(a.state as i64), a.ctime));
    let chosen = candidates.remove(0);
    for dup in &candidates {
        warn!(
            "剧目 {:?} 存在重复稿件 {}（state={}），本次只往 {} 里追加",
            chosen.title, dup.bvid, dup.state, chosen.bvid
        );
    }
    chosen
}

/// 剧目名 -> 该剧目认定的那一个稿件。纯函数，方便单测。
fn choose_archives(archives: Vec<Archive>) -> HashMap<String, Archive> {
    let mut by_title: HashMap<String, Vec<Archive>> = HashMap::new();
    for a in archives {
        by_title
            .entry(title_of_archive(&a.title))
            .or_default()
            .push(a);
    }
    by_title
        .into_iter()
        .map(|(t, v)| (t, pick_archive(v)))
        .collect()
}

/// 去掉稿件里已经有的分P。纯函数，方便单测。
fn drop_existing(parts: Vec<VideoUrl>, existing: &[String]) -> Vec<VideoUrl> {
    parts
        .into_iter()
        .filter(|p| !existing.contains(&truncate_title(&p.name)))
        .collect()
}

pub async fn build_plan(settings: &Settings, ledger: &Ledger) -> Result<Vec<Task>> {
    // 片源列表带本地缓存：上游一次返回两千多条，每次运行都去拉容易被限流
    let catalog = xmtv_api::Videos::cached(CATALOG_CACHE, settings.catalog_ttl).await?;
    let groups = catalog.videos;
    info!(
        "XMTV 片源共 {} 部戏（缓存时间 {}）",
        groups.len(),
        chrono::DateTime::from_timestamp(catalog.last_update, 0)
            .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "未知".into())
    );

    // 只跑一部戏时按关键词搜就够了。翻完整的稿件列表在稿件上千的账号上要二十多页，
    // 很容易撞上投稿中心的 -702 限流，而做端到端验证时根本用不着整份列表。
    let archives = match &settings.only_title {
        Some(kw) => bili::search_archives(kw).await?,
        None => bili::list_archives().await?,
    };
    let chosen = choose_archives(archives);

    // 先做本地筛选，再决定要不要为这部戏去查一次 b 站接口
    let mut candidates: Vec<(String, String, Vec<VideoUrl>)> = Vec::new();
    let mut skipped_dead = 0usize;
    for group in groups {
        if let Some(f) = &settings.only_title
            && !group.title.contains(f.as_str())
        {
            continue;
        }
        let archive = chosen.get(&group.title);

        // state=-4 之类是"没发出去"，这种稿件里的内容并没有真的上线，
        // 重新投一个才是对的——多投几次总有能过审的。
        // 只有明确要求时才跳过（XIDOWN_SKIP_DEAD=1）。
        if let Some(a) = archive
            && bili::state_is_dead(a.state as i64)
        {
            if settings.skip_dead {
                warn!(
                    "剧目 {} 的稿件 {} 未发出（state={} {}），按 XIDOWN_SKIP_DEAD 跳过",
                    group.title, a.bvid, a.state, a.state_desc
                );
                skipped_dead += 1;
                continue;
            }
            info!(
                "剧目 {} 的稿件 {} 未发出（state={} {}），重新投一个",
                group.title, a.bvid, a.state, a.state_desc
            );
        }

        // 稿件没发出去就当作还没有稿件，走新投稿流程
        let bv = match archive {
            Some(a) if !bili::state_is_dead(a.state as i64) => a.bvid.clone(),
            _ => String::new(),
        };

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
    if skipped_dead > 0 {
        warn!("因为稿件已失效而跳过 {skipped_dead} 部戏");
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
            let parts = drop_existing(parts, &existing);
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

    /// 审核中(-30)/待审(-1) 是正常的中间状态，仍然能追加分P，
    /// 不能和"被锁定(-4)"一样被排到后面——否则会去给一部其实好好的戏重新投稿。
    #[test]
    fn test_pick_archive_prefers_under_review_over_locked() {
        let got = pick_archive(vec![
            archive("BV_locked", -4, 100),
            archive("BV_reviewing", -30, 200),
        ]);
        assert_eq!(got.bvid, "BV_reviewing");

        let got = pick_archive(vec![
            archive("BV_locked", -4, 100),
            archive("BV_pending", -1, 200),
        ]);
        assert_eq!(got.bvid, "BV_pending");
    }

    /// 同一个剧目名有多个稿件时只能认一个——旧版会往每一个里都塞一遍。
    #[test]
    fn test_choose_archives_collapses_duplicates() {
        let mut a = archive("BV_dup", 0, 300);
        a.title = "甲 斗阵来看戏".into();
        let mut b = archive("BV_first", 0, 100);
        b.title = "甲 斗阵来看戏".into();
        let mut c = archive("BV_other", 0, 100);
        c.title = "乙 斗阵来看戏".into();

        let chosen = choose_archives(vec![a, b, c]);
        assert_eq!(chosen.len(), 2, "两个剧目名只应该有两个条目");
        assert_eq!(chosen["甲"].bvid, "BV_first");
        assert_eq!(chosen["乙"].bvid, "BV_other");
    }

    fn url(name: &str, id: u64) -> VideoUrl {
        VideoUrl {
            title: "甲".into(),
            name: name.into(),
            url: String::new(),
            time: id as u128,
            id,
        }
    }

    #[test]
    fn test_drop_existing() {
        let parts = vec![url("甲（1）", 1), url("甲（2）", 2), url("甲（3）", 3)];
        let existing = vec!["甲（1）".to_string(), "甲（3）".to_string()];
        let got = drop_existing(parts, &existing);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "甲（2）");
    }

    /// 没发出去的稿件不能被当成"已经有稿件了"：上层正是因为它没发出去才要重投，
    /// 要是把它认成正主，分P 就会被追加回那个坏稿件，重投等于白做。
    #[test]
    fn test_dead_archive_is_not_treated_as_usable() {
        let mut dead = archive("BV_dead", -4, 100);
        dead.title = "甲 斗阵来看戏".into();
        let chosen = choose_archives(vec![dead]);
        let picked = &chosen["甲"];
        assert!(
            bili::state_is_dead(picked.state as i64),
            "-4 必须被判定为没发出去，这样才会走重新投稿"
        );
    }

    /// 超长标题在 b 站上是被截断存的，比对时必须用同样的截断规则，
    /// 否则每次运行都会觉得"还没传过"而重复上传。
    #[test]
    fn test_drop_existing_matches_truncated_titles() {
        let long: String = "字".repeat(200);
        let parts = vec![url(&long, 1)];
        let existing = vec![truncate_title(&long)];
        assert!(drop_existing(parts, &existing).is_empty());
    }
}
