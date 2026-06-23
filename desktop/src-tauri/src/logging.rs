//! 统一日志装配。
//!
//! 装一个全局 `tracing` subscriber,把两路日志收进**同一个**全局日志文件,用每行的
//! `target` 字段区分来源:
//!
//! - **daemon(本地数据层)** —— `mc-core` 与 Tauri 命令层的 `tracing` 事件,target 是它们的
//!   Rust 模块路径(如 `mc_core::modpack::import::engine`)。这些事件原本因为没装 subscriber
//!   而被丢弃,现在被捕获。
//! - **client(前端 webview)** —— 经 `client_log` / `log_boot` 命令转发回来的日志,target 固定
//!   为 `client`,所以在日志里以 `client:` 开头,一眼区分于 daemon 的模块路径。
//!
//! 日志写到 `<data_dir>/logs/mc-launcher.log`(按日滚动)。debug 构建同时镜像到 stderr。
//! `MC_LOG`(或 `RUST_LOG`)可覆盖默认过滤级别,例如 `MC_LOG=mc_core=trace`。

use std::path::Path;

use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

/// 默认日志过滤:整体 info,本地数据层与命令层放到 debug,便于排查。
fn default_filter() -> EnvFilter {
    EnvFilter::try_from_env("MC_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info,mc_core=debug,mc_launcher_desktop_lib=debug"))
}

/// 装配全局 subscriber,返回必须**持有到进程退出**的写入守卫(`non_blocking` 在它析构时刷盘)。
/// 任何一步失败都尽力降级(只装控制台)而不让应用起不来;返回 `None` 表示未启用文件日志。
pub fn init(logs_dir: &Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    // 注:各 fmt layer 必须在 `.with(...)` 里就地构造,不能先 `let` 出来再复用 —— 那样会把它的
    // 泛型 subscriber 类型 pin 死,导致与下一层叠加时类型不匹配(编译期 Layer trait 报错)。

    // 建不出日志目录就只装控制台一路(stderr),不阻断启动。
    if mc_core::paths::ensure_dir(logs_dir).is_err() {
        let _ = tracing_subscriber::registry()
            .with(default_filter())
            .with(
                tracing_subscriber::fmt::layer()
                    .with_target(true)
                    .with_writer(std::io::stderr),
            )
            .try_init();
        return None;
    }

    let appender = tracing_appender::rolling::daily(logs_dir, "mc-launcher.log");
    let (file_writer, guard) = tracing_appender::non_blocking(appender);

    // 文件一路(按日滚动,无 ANSI)+ stderr 一路(开发期直接看)。两路都打印 target 以区分
    // client(前端,target=client)与 daemon(本地数据层,target=模块路径)。
    let _ = tracing_subscriber::registry()
        .with(default_filter())
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_ansi(false)
                .with_writer(file_writer),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_writer(std::io::stderr),
        )
        .try_init();

    Some(guard)
}
