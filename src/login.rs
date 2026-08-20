//! 扫码登录，生成 / 刷新 `cookies.json`。
//!
//! b 站的 token 过期后只能重新登录，而登录必须由人拿手机确认，没有别的办法。
//! 用 `XIDOWN_LOGIN=1 cargo run` 进入这个模式，终端里会画出二维码，
//! 用 bilibili App 扫一下即可。

use crate::config::{COOKIE_FILE, PROXY};
use anyhow::{Context, Result, anyhow};
use biliup::credential::Credential;
use log::info;
use qrcode::QrCode;
use qrcode::render::unicode;

pub async fn login() -> Result<()> {
    let credential = Credential::new(PROXY);

    let value = credential
        .get_qrcode()
        .await
        .map_err(|e| anyhow!("获取二维码失败: {e}"))?;
    let url = value["data"]["url"]
        .as_str()
        .ok_or_else(|| anyhow!("二维码接口没有返回 url: {value}"))?;

    let code = QrCode::new(url.as_bytes()).context("生成二维码失败")?;
    let rendered = code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();

    println!("\n请用 bilibili App 扫描下面的二维码并确认登录：\n");
    println!("{rendered}");
    println!("如果终端里的二维码显示不正常，也可以直接在 App 里打开这个链接：\n{url}\n");
    println!("等待扫码确认中……（确认后会自动继续）");

    let info = credential
        .login_by_qrcode(value)
        .await
        .map_err(|e| anyhow!("扫码登录失败: {e}"))?;

    // login_by_qrcode 只返回 LoginInfo，不会自己落盘，这里得手动写
    let file = std::fs::File::create(COOKIE_FILE)
        .context(format!("创建 {COOKIE_FILE} 失败"))?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(file), &info)
        .context(format!("写入 {COOKIE_FILE} 失败"))?;

    info!("登录成功，凭据已写入 {COOKIE_FILE} (mid={})", info.token_info.mid);
    println!("\n登录成功，凭据已保存到 {COOKIE_FILE}，现在可以正常运行了。");
    Ok(())
}
