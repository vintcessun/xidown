//! 集中放置所有可调参数。
//!
//! 需要临时改变行为（例如做端到端验证时只跑一部戏）时用环境变量覆盖，
//! 不用改代码重新编译。

use std::path::PathBuf;

/// b 站登录 cookie
pub const COOKIE_FILE: &str = "cookies.json";
/// 本地上传台账，用来避免重复上传
pub const LEDGER_FILE: &str = "uploaded.json";
/// 节目名，同时用于标题拼接和从稿件标题里反推剧目名
pub const KEYWORD: &str = "斗阵来看戏";
/// 投稿分区：戏曲
pub const TID: u16 = 180;
pub const TAG: &str = "戏曲,斗阵来看戏";
pub const SOURCE: &str = "https://2020.xmtv.cn/search/?search_text=斗阵来看戏";
pub const DESC: &str = "自传给家里老人看方便";
/// b 站标题 / 分P 标题的字符上限
pub const MAX_TITLE_CHARS: usize = 80;
/// 查询稿件列表时使用的状态过滤
pub const ARCHIVE_STATUS: &str = "is_pubing,pubed,not_pubed";

pub const PROXY: Option<&str> = None;

fn env_str(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.trim().is_empty())
}

fn env_num<T: std::str::FromStr>(key: &str, default: T) -> T {
    env_str(key)
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_flag(key: &str) -> bool {
    matches!(
        env_str(key).as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

#[derive(Debug, Clone)]
pub struct Settings {
    /// 同时处理几部戏
    pub concurrency: usize,
    /// 单个文件上传时的并发分片数
    pub upload_limit: usize,
    /// 临时视频文件存放目录
    pub work_dir: PathBuf,
    /// 只处理剧目名包含该子串的条目（端到端验证用）
    pub only_title: Option<String>,
    /// 最多处理几部戏
    pub max_archives: Option<usize>,
    /// 每部戏最多上传几个分P
    pub max_parts: Option<usize>,
    /// 只输出计划，不下载也不上传
    pub dry_run: bool,
}

impl Settings {
    pub fn from_env() -> Self {
        Self {
            concurrency: env_num("XIDOWN_CONCURRENCY", 4usize).max(1),
            upload_limit: env_num("XIDOWN_UPLOAD_LIMIT", 10usize).max(1),
            work_dir: env_str("XIDOWN_WORK_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("work")),
            only_title: env_str("XIDOWN_ONLY_TITLE"),
            max_archives: env_str("XIDOWN_MAX_ARCHIVES").and_then(|s| s.parse().ok()),
            max_parts: env_str("XIDOWN_MAX_PARTS").and_then(|s| s.parse().ok()),
            dry_run: env_flag("XIDOWN_DRY_RUN"),
        }
    }
}

/// 按字符（不是字节）截断到 b 站允许的长度。
///
/// 必须和 biliup 从文件名推断分P 标题时的行为完全一致：历史上的分P 标题就是那么
/// 生成的，去重时拿不同的规则去比对，边界长度的标题会被判成"还没传过"而重复上传。
/// 注意 biliup 那边是 `if len >= 80 { truncate_title(s, 80) }`，而
/// `truncate_title` 内部又是 `if len <= 80 { 原样返回 }`，两者叠加之后
/// 正好 80 字是**不截断**的，所以这里用 `<=`。
pub fn truncate_title(title: &str) -> String {
    if title.chars().count() <= MAX_TITLE_CHARS {
        return title.to_string();
    }
    let kept: String = title.chars().take(MAX_TITLE_CHARS - 3).collect();
    format!("{kept}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 复刻 biliup `Parcel::upload` 里推断分P 标题的那段逻辑，
    /// 用来锁住我们和它的行为一致。
    fn biliup_part_title(name: &str) -> String {
        if name.chars().count() >= MAX_TITLE_CHARS {
            biliup::bilibili::Video::truncate_title(name, MAX_TITLE_CHARS)
        } else {
            name.to_string()
        }
    }

    #[test]
    fn test_truncate_title_matches_biliup() {
        for len in [1usize, 10, 78, 79, 80, 81, 200] {
            let s: String = "字".repeat(len);
            assert_eq!(
                truncate_title(&s),
                biliup_part_title(&s),
                "长度 {len} 时两边的截断结果必须一致"
            );
        }
        assert_eq!(truncate_title("白蛇传"), "白蛇传");
        assert_eq!(
            truncate_title(&"字".repeat(200)).chars().count(),
            MAX_TITLE_CHARS
        );
    }
}
