mod bili;
mod catalog;
mod config;
mod ledger;
mod login;
mod worker;

use anyhow::{Context, Result};
use chrono::Local;
use config::{LEDGER_FILE, Settings};
use fern::colors::{Color, ColoredLevelConfig};
use indicatif::MultiProgress;
use ledger::Ledger;
use log::{error, info, warn};
use once_cell::sync::Lazy;
use std::fs::{File, create_dir_all};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use worker::Ctx;

static MULTI_PROGRESS: Lazy<Arc<MultiProgress>> = Lazy::new(|| Arc::new(MultiProgress::new()));

#[tokio::main]
async fn main() -> Result<()> {
    // 日志必须先装好，否则前面几步的输出全丢了
    let log_path = set_logger()?;
    info!("日志写入 {log_path}");

    // token 过期时唯一的出路是重新扫码登录，单独走一个模式
    if std::env::var("XIDOWN_LOGIN").is_ok_and(|v| !v.trim().is_empty() && v != "0") {
        return login::login().await;
    }

    let settings = Settings::from_env();
    info!("运行参数 {settings:?}");
    create_dir_all(&settings.work_dir)
        .context(format!("创建工作目录 {} 失败", settings.work_dir.display()))?;

    // 先确认账号能用，别等下载完几个 G 才发现 cookie 过期
    bili::probe_login().await?;

    let ledger = Arc::new(Ledger::load(LEDGER_FILE).await?);
    let tasks = catalog::build_plan(&settings, &ledger).await?;

    let total_parts: usize = tasks.iter().map(|t| t.parts.len()).sum();
    info!(
        "计划：{} 部戏，共 {} 个分P 需要上传",
        tasks.len(),
        total_parts
    );
    for t in &tasks {
        info!(
            "  {} [{}] {} 个分P: {}",
            t.title,
            if t.bv.is_empty() { "新投稿" } else { &t.bv },
            t.parts.len(),
            t.parts
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }

    if tasks.is_empty() {
        info!("没有需要上传的内容，结束");
        return Ok(());
    }
    if settings.dry_run {
        info!("XIDOWN_DRY_RUN 已开启，只输出计划，不下载也不上传");
        return Ok(());
    }

    let ctx = Arc::new(Ctx {
        settings: settings.clone(),
        ledger: ledger.clone(),
        multi: MULTI_PROGRESS.clone(),
        submit_blocked: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });

    // 旧版是 threadpool 里给每个任务单独 build 一个多线程 tokio runtime，
    // 4 个任务就有 4 套线程池。这里直接用主 runtime 上的任务 + 信号量限流。
    let sem = Arc::new(Semaphore::new(settings.concurrency));
    let mut set = JoinSet::new();
    for task in tasks {
        let ctx = ctx.clone();
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.expect("信号量不会被关闭");
            let title = task.title.clone();
            let parts = task.parts.len();
            match worker::run_task(ctx, task).await {
                Ok(()) => {
                    info!("{title} 完成，共 {parts} 个分P");
                    Ok(())
                }
                Err(e) => {
                    error!("{title} 失败: {e:#}");
                    Err(title)
                }
            }
        });
    }

    let mut failed = Vec::new();
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(())) => {}
            Ok(Err(title)) => failed.push(title),
            Err(e) => {
                error!("任务 panic: {e}");
                failed.push(format!("<panic: {e}>"));
            }
        }
    }

    if failed.is_empty() {
        info!("全部完成");
        Ok(())
    } else {
        warn!("以下 {} 部戏没有处理完: {}", failed.len(), failed.join(", "));
        // 部分失败不算整体失败，下次运行会靠台账续上
        Ok(())
    }
}

fn set_logger() -> Result<String> {
    let colors = ColoredLevelConfig::new()
        .debug(Color::Cyan)
        .info(Color::Green)
        .warn(Color::Yellow)
        .error(Color::Red);

    let console_dispatch = fern::Dispatch::new()
        .format(move |out, message, record| {
            // 打日志前先把进度条收起来，免得两边互相覆盖
            MULTI_PROGRESS.suspend(|| {
                out.finish(format_args!(
                    "[{}] [{}] [{}] {}",
                    Local::now().format("%Y-%m-%d %H:%M:%S"),
                    colors.color(record.level()),
                    record.target(),
                    message
                ))
            });
        })
        .level(log::LevelFilter::Info)
        .chain(std::io::stdout());

    let log_dir = "./log";
    create_dir_all(log_dir)?;
    let log_filename = format!("{log_dir}/{}.log", Local::now().format("%Y%m%d-%H%M%S"));

    let file_dispatch = fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "[{}] [{}] [{}] {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(File::create(&log_filename)?);

    fern::Dispatch::new()
        .chain(console_dispatch)
        .chain(file_dispatch)
        .apply()?;

    // biliup 内部用的是 tracing，不接进来的话上传失败时看不到任何细节。
    // 单独写一个文件，避免它的 INFO 刷屏把进度条冲掉。
    let tracing_filename = format!("{log_dir}/{}.biliup.log", Local::now().format("%Y%m%d-%H%M%S"));
    if let Ok(f) = File::create(&tracing_filename) {
        let _ = tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(f))
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .try_init();
    }

    Ok(log_filename)
}
