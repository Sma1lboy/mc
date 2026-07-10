use super::*;

// --- theme persistence ----------------------------------------------------

fn theme_path() -> PathBuf {
    data_dir().join("theme.json")
}

#[tauri::command]
#[specta::specta]
pub fn get_theme() -> CmdResult<ThemeConfig> {
    match std::fs::read_to_string(theme_path()) {
        Ok(s) => serde_json::from_str(&s).map_err(err),
        Err(_) => Ok(ThemeConfig::default()),
    }
}

#[tauri::command]
#[specta::specta]
pub fn set_theme(cfg: ThemeConfig) -> CmdResult<()> {
    let _ = paths::ensure_dir(&data_dir());
    let s = serde_json::to_string_pretty(&cfg).map_err(err)?;
    std::fs::write(theme_path(), s).map_err(err)
}

/// 前端 webview 把启动/错误信息报到这里;经全局 tracing 落进统一日志(`[client]` 前缀)。
#[tauri::command]
#[specta::specta]
pub fn log_boot(msg: String) {
    tracing::info!(target: "client", "{msg}");
}

/// 前端统一日志入口:把 webview 的日志按级别转发到全局日志文件(`[client]` 前缀),
/// 与本地数据层(`[daemon]`)的日志汇到同一处,方便对照排查。
/// level ∈ `error` / `warn` / `info` / `debug`(其它按 info 处理)。
#[tauri::command]
#[specta::specta]
pub fn client_log(level: String, message: String) {
    match level.as_str() {
        "error" => tracing::error!(target: "client", "{message}"),
        "warn" => tracing::warn!(target: "client", "{message}"),
        "debug" => tracing::debug!(target: "client", "{message}"),
        _ => tracing::info!(target: "client", "{message}"),
    }
}

/// 返回全局日志目录(`<data_dir>/logs`,必要时创建),前端用 shell 打开它。
#[tauri::command]
#[specta::specta]
pub fn open_logs_dir() -> CmdResult<String> {
    let dir = mc_core::paths::logs_dir(&data_dir());
    paths::ensure_dir(&dir).map_err(err)?;
    Ok(dir.to_string_lossy().into_owned())
}

/// 读取最新日志文件的末尾若干行,供应用内日志查看器。日志按日滚动(文件名形如
/// `mc-launcher.log.<日期>`),取修改时间最新的那个;有界读取(末尾最多 512KiB)避免大日志卡 UI。
#[tauri::command]
#[specta::specta]
pub fn read_log_tail(lines: usize) -> CmdResult<String> {
    use std::io::{Read, Seek, SeekFrom};

    let dir = mc_core::paths::logs_dir(&data_dir());
    let newest = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("mc-launcher.log"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p);
    let Some(path) = newest else {
        return Ok(String::new());
    };

    const MAX_BYTES: u64 = 512 * 1024;
    let mut f = std::fs::File::open(&path).map_err(err)?;
    let len = f.metadata().map_err(err)?.len();
    let start = len.saturating_sub(MAX_BYTES);
    f.seek(SeekFrom::Start(start)).map_err(err)?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).map_err(err)?;
    let text = String::from_utf8_lossy(&bytes);
    // 从中途开始读时丢掉可能不完整的首行。
    let text: &str = if start > 0 {
        text.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
    } else {
        &text
    };

    let cap = lines.clamp(1, 5000);
    let mut collected: Vec<&str> = text.lines().rev().take(cap).collect();
    collected.reverse();
    Ok(collected.join("\n"))
}

/// 读取全局设置(下载源/并发/默认内存/Java 路径/语言…)。缺失/损坏回退默认。
#[tauri::command]
#[specta::specta]
pub fn get_settings() -> CmdResult<mc_core::settings::GlobalSettings> {
    mc_core::settings::GlobalSettings::load(&data_dir()).map_err(err)
}

/// 持久化全局设置(原子写 settings.json)。下载相关项下次构造下载器即生效。
#[tauri::command]
#[specta::specta]
pub fn set_settings(settings: mc_core::settings::GlobalSettings) -> CmdResult<()> {
    settings.save(&data_dir()).map_err(err)
}

/// 当前生效的「显示社交 UI」(kobeMC 账号 / 领域 / 好友)开关:用户显式设置优先,
/// 否则按部署场景默认(便携·和实例同级 → 关;桌面独立版 → 开)。
#[tauri::command]
#[specta::specta]
pub fn social_enabled() -> CmdResult<bool> {
    Ok(settings_global()
        .social_enabled
        .unwrap_or_else(|| !mc_core::paths::is_portable_deployment(&exe_dir())))
}

