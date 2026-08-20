//! 本地上传台账。
//!
//! 只靠 b 站接口去重是不够的：稿件刚提交时 `archive/view` 里往往还查不到新的分P，
//! 紧接着再跑一次就会把同一集重新传一遍。台账在**每次上传成功后立刻落盘**，
//! 于是即使中途被 Ctrl-C 或者断网，下一次运行也不会重复上传。

use anyhow::{Context, Result};
use chrono::Utc;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;
use xmtv_api::VideoUrl;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// 传到了哪个稿件
    pub bv: String,
    /// 在稿件里的分P标题
    pub part_title: String,
    /// 剧目名
    pub title: String,
    pub uploaded_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Data {
    #[serde(default)]
    parts: HashMap<String, Entry>,
}

pub struct Ledger {
    path: PathBuf,
    inner: Mutex<Data>,
}

/// 分P 的去重键。优先用 XMTV 的稳定 id，老数据没有 id 时退回标题。
pub fn part_key(v: &VideoUrl) -> String {
    if v.id != 0 {
        format!("id:{}", v.id)
    } else {
        format!("name:{}", v.name)
    }
}

impl Ledger {
    pub async fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let data = match tokio::fs::read_to_string(&path).await {
            Ok(s) => match serde_json::from_str::<Data>(&s) {
                Ok(d) => d,
                Err(e) => {
                    // 台账坏了不该拖垮整次运行，最坏情况只是退化成靠 b 站接口去重
                    warn!("台账 {} 解析失败({e})，按空台账处理", path.display());
                    Data::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Data::default(),
            Err(e) => return Err(e).context(format!("读取台账 {} 失败", path.display())),
        };
        info!("上传台账加载完成，已记录 {} 个分P", data.parts.len());
        Ok(Self {
            path,
            inner: Mutex::new(data),
        })
    }

    /// 这个分P 是否已经传过了（可选地要求传到了指定稿件）。
    pub async fn contains(&self, key: &str, expect_bv: Option<&str>) -> Option<Entry> {
        let guard = self.inner.lock().await;
        let entry = guard.parts.get(key)?;
        match expect_bv {
            // 记录里的 bv 和目标稿件不一致，说明目标稿件换了，得重新传
            Some(bv) if !bv.is_empty() && entry.bv != bv => None,
            _ => Some(entry.clone()),
        }
    }

    /// 记录一个分P 并立刻落盘。
    pub async fn record(&self, key: String, entry: Entry) -> Result<()> {
        let mut guard = self.inner.lock().await;
        guard.parts.insert(key, entry);
        Self::save(&self.path, &guard).await
    }

    /// 某个稿件被打回、需要重新投稿时，清掉它名下的所有记录。
    pub async fn forget_archive(&self, bv: &str) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let before = guard.parts.len();
        guard.parts.retain(|_, e| e.bv != bv);
        if guard.parts.len() != before {
            warn!(
                "稿件 {bv} 需要重新投稿，已清除台账中 {} 条记录",
                before - guard.parts.len()
            );
            Self::save(&self.path, &guard).await?;
        }
        Ok(())
    }

    /// 先写临时文件再 rename，避免写到一半掉电留下半个 json。
    async fn save(path: &Path, data: &Data) -> Result<()> {
        let json = serde_json::to_string_pretty(data)?;
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, json.as_bytes())
            .await
            .context(format!("写入台账临时文件 {} 失败", tmp.display()))?;
        tokio::fs::rename(&tmp, path)
            .await
            .context(format!("替换台账 {} 失败", path.display()))?;
        Ok(())
    }
}

pub fn now_ts() -> i64 {
    Utc::now().timestamp()
}
