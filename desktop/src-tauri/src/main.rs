// 发布构建下隐藏 Windows 控制台窗口(debug 下保留以便看日志)。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// 二进制入口只做一件事:把控制权交给 lib 的 run()。
// 真正的 Tauri Builder 装配在 lib.rs,这样既能产出 cdylib(移动端)也能跑桌面端。
fn main() {
    mc_launcher_desktop_lib::run()
}
