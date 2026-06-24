//! 截图画廊(开发用,仅 macOS)。`MC_GALLERY=1` 时前端逐页驱动:每切到一页就调
//! [`gallery_capture`] 用系统 `screencapture` 抓「main」原生窗口存盘,最后
//! [`gallery_build`] 生成 `index.html` 网格画廊。无 launcher 逻辑,纯平台截图 + 落盘。

use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::time::Duration;

use serde::Deserialize;
use tauri::{AppHandle, Manager};

use mc_core::paths;

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn data_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    paths::resolve_data_dir(&exe_dir)
}

fn gallery_dir() -> PathBuf {
    data_dir().join("gallery")
}

/// 文件名安全化:只留字母数字与 - _,其余换成 _。
fn slug(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// 是否处于画廊模式(环境变量 `MC_GALLERY` 非空且非 "0")。前端据此决定是否自动跑截图流程。
#[tauri::command]
#[specta::specta]
pub fn gallery_enabled() -> CmdResult<bool> {
    // 返回 Result(而非裸 bool):api 代理对所有命令按 Result 信封统一解包,裸值会被解成 undefined。
    Ok(std::env::var("MC_GALLERY")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false))
}

/// 抓「main」窗口当前画面到 `<data_dir>/gallery/<name>.png`,返回文件绝对路径。
#[tauri::command]
#[specta::specta]
pub fn gallery_capture(app: AppHandle, name: String) -> CmdResult<String> {
    capture_window(&app, &name)
}

#[cfg(target_os = "macos")]
fn capture_window(app: &AppHandle, name: &str) -> CmdResult<String> {
    let win = app.get_webview_window("main").ok_or("未找到 main 窗口")?;
    // 聚焦让内容确保已渲染;但截图按窗口 id 抓自身后备缓冲,不依赖 z 序/遮挡,所以即便
    // 前台是终端也只会抓到本窗口(区域截图会抓错窗口,故弃用)。
    let _ = win.set_focus();
    std::thread::sleep(Duration::from_millis(200));

    let window_id = ns_window_number(&win)?;

    let dir = gallery_dir();
    std::fs::create_dir_all(&dir).map_err(err)?;
    let out = dir.join(format!("{}.png", slug(name)));

    // -x 静音,-o 不带窗口阴影,-l<id> 按 CGWindowID 抓指定窗口。
    let status = std::process::Command::new("screencapture")
        .arg("-x")
        .arg("-o")
        .arg(format!("-l{window_id}"))
        .arg(&out)
        .status()
        .map_err(err)?;
    if !status.success() {
        return Err(format!("screencapture 退出码 {status}(可能缺少「屏幕录制」权限)"));
    }
    Ok(out.to_string_lossy().into_owned())
}

/// 取窗口的 CGWindowID:NSWindow 的 `windowNumber`(供 `screencapture -l` 用)。
#[cfg(target_os = "macos")]
fn ns_window_number(win: &tauri::WebviewWindow) -> CmdResult<isize> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    let ns_window = win.ns_window().map_err(err)? as *mut AnyObject;
    if ns_window.is_null() {
        return Err("ns_window 为空".into());
    }
    // windowNumber 返回 NSInteger(isize);>0 即为有效的 CGWindowID。
    let number: isize = unsafe { msg_send![ns_window, windowNumber] };
    if number <= 0 {
        return Err(format!("无效的 windowNumber: {number}"));
    }
    Ok(number)
}

#[cfg(not(target_os = "macos"))]
fn capture_window(_app: &AppHandle, _name: &str) -> CmdResult<String> {
    Err("画廊截图当前仅支持 macOS".into())
}

/// 一张截图的元信息:`name` 对应 `<name>.png`,`title` 是画廊里显示的标题。
#[derive(Deserialize, specta::Type)]
pub struct GalleryShot {
    pub name: String,
    pub title: String,
}

/// 用已抓好的截图生成 `<data_dir>/gallery/index.html` 网格画廊,返回其绝对路径。
#[tauri::command]
#[specta::specta]
pub fn gallery_build(app: AppHandle, shots: Vec<GalleryShot>) -> CmdResult<String> {
    // 收尾:取消置顶,恢复正常窗口行为。
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_always_on_top(false);
    }

    let dir = gallery_dir();
    std::fs::create_dir_all(&dir).map_err(err)?;

    let cards: String = shots
        .iter()
        .map(|s| {
            format!(
                "<figure><div class=\"frame\"><img src=\"{file}.png\" loading=\"lazy\" alt=\"{title}\"/></div><figcaption>{title}</figcaption></figure>",
                file = slug(&s.name),
                title = html_escape(&s.title),
            )
        })
        .collect();

    let html = format!(
        "<!doctype html>\n<html lang=\"zh-CN\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>kobeMC · 页面画廊</title>\
<style>\
:root{{color-scheme:dark}}\
*{{box-sizing:border-box}}\
body{{margin:0;background:#0f1115;color:#e7e9ee;font:14px/1.5 -apple-system,system-ui,'PingFang SC',sans-serif}}\
header{{padding:28px 32px 8px}}\
h1{{margin:0;font-size:20px;font-weight:650}}\
.sub{{color:#8b90a0;font-size:13px;margin-top:4px}}\
.grid{{display:grid;grid-template-columns:repeat(auto-fill,minmax(440px,1fr));gap:24px;padding:24px 32px 48px}}\
figure{{margin:0}}\
.frame{{border:1px solid #23262f;border-radius:12px;overflow:hidden;background:#000;box-shadow:0 8px 28px rgba(0,0,0,.45)}}\
.frame img{{display:block;width:100%;height:auto}}\
figcaption{{margin-top:10px;color:#aeb3c2;font-size:13px;text-align:center}}\
</style></head><body>\
<header><h1>kobeMC · 页面画廊</h1><div class=\"sub\">{count} 张 · 真实 Tauri 窗口截图</div></header>\
<main class=\"grid\">{cards}</main></body></html>",
        count = shots.len(),
        cards = cards,
    );

    let out = dir.join("index.html");
    std::fs::write(&out, html).map_err(err)?;
    // gallery.sh 靠这条标记判定流程跑完(webview 的 console.log 不进程 stdout,tracing 才进日志/stderr)。
    tracing::info!(target: "daemon", "画廊已生成: {}", out.display());
    Ok(out.to_string_lossy().into_owned())
}

/// 最小 HTML 转义,够覆盖标题文本。
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
