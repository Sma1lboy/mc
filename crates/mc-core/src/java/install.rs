//! Java 运行时的自动下载与安装。
//!
//! 与 [`detect`](super::detect) 互补: detect 负责"看系统已有什么", 本模块负责
//! "系统没有合适的 Java 时, 去网上拉一个回来"。
//!
//! 我们用 **Adoptium (Eclipse Temurin)** 的直链 API, 它跨平台、稳定、且无需先解析
//! Mojang 的 runtime manifest:
//!
//! ```text
//! https://api.adoptium.net/v3/binary/latest/{major}/ga/{os}/{arch}/jre/hotspot/normal/eclipse
//! ```
//!
//! 该 URL 会 302 重定向到具体的归档文件:
//!   - mac / linux → `.tar.gz`
//!   - windows     → `.zip`
//!
//! 安装流程:
//!   1. 拼出 Adoptium 直链 ([`adoptium_url`]).
//!   2. 自建一个 *默认跟随重定向* 的 `reqwest::Client` 下载字节到归档文件。
//!   3. 按平台解压 (mac/linux 用 `flate2` + `tar`; windows 用 `zip`).
//!   4. 在解压目录里找到 `java` 可执行文件 ([`find_java_in`]), 返回其绝对路径。
//!   5. 删除下载的归档。
//!
//! 解压目标固定为 `dest_root/jre-{major}/` ([`required_jre_dir`]); 若该目录里已能找到
//! `java`, 则直接复用, 跳过整个下载流程 (幂等)。

use std::path::{Path, PathBuf};

use crate::error::{CoreError, IoResultExt, Result};

/// 把当前平台映射为 Adoptium API 的操作系统标识。
///
/// Adoptium 取值: `mac` / `linux` / `windows`。其它平台目前无官方 Temurin 构建,
/// 这里仍尽量给出一个合理值 (退回 `linux`), 调用方下载失败时会自然报错。
fn adoptium_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        // linux 及其它类 unix。
        "linux"
    }
}

/// 把当前平台映射为 Adoptium API 的 CPU 架构标识。
///
/// Adoptium 取值: `x64` (即 x86_64) / `aarch64` (即 arm64)。
fn adoptium_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        // x86_64 → "x64"; 其余架构退回 x64 (最常见), 失败时由下载层报错。
        "x64"
    }
}

/// 拼出 Adoptium "最新 GA JRE" 的二进制直链。
///
/// 抽成纯函数便于单测 (不联网)。`os` / `arch` 由 [`adoptium_os`] / [`adoptium_arch`]
/// 提供, 但这里接收参数, 这样测试可以覆盖所有平台组合。
fn adoptium_url(major: u8, os: &str, arch: &str) -> String {
    format!(
        "https://api.adoptium.net/v3/binary/latest/{major}/ga/{os}/{arch}/jre/hotspot/normal/eclipse"
    )
}

/// 当前平台上 java 可执行文件名 (`java.exe` on Windows, 否则 `java`)。
fn java_exe_name() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// 返回某个大版本对应的解压目标目录: `dest_root/jre-{major}`。
///
/// 这是个稳定的、可预测的路径: 上层可以先 [`find_java_in`] 探一下, 已装好就跳过下载。
pub fn required_jre_dir(dest_root: &Path, major: u8) -> PathBuf {
    dest_root.join(format!("jre-{major}"))
}

/// 在一个已解压的 JRE/JDK 目录树里寻找 `java` 可执行文件, 返回其绝对路径。
///
/// 兼容两种布局:
///   - macOS 的 `.tar.gz` 解出来是 `jdk-XX.app/Contents/Home/bin/java` (或顶层直接
///     `<root>/Contents/Home/bin/java`).
///   - linux / windows 是 `<root>/bin/java[.exe]`.
///
/// Adoptium 归档通常会把所有内容套在一个顶层目录里 (如 `jdk-17.0.14+7-jre/`), 所以
/// 我们既检查 `dir` 本身, 也检查它的每个直接子目录。找不到返回 `None`。
pub fn find_java_in(dir: &Path) -> Option<PathBuf> {
    // 先在传入目录本身按已知相对路径找。
    if let Some(p) = java_in_layout(dir) {
        return Some(p);
    }
    // 再扫一层子目录 (Adoptium 的顶层套壳目录, 名字带版本号不可预测)。
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let child = entry.path();
        if !child.is_dir() {
            continue;
        }
        if let Some(p) = java_in_layout(&child) {
            return Some(p);
        }
    }
    None
}

/// 在单个候选根目录下按两种已知布局探测 `java`, 命中即返回规范化绝对路径。
fn java_in_layout(root: &Path) -> Option<PathBuf> {
    let candidates = [
        // linux / windows 直接布局。
        root.join("bin").join(java_exe_name()),
        // macOS .app / Contents/Home 布局。
        root.join("Contents").join("Home").join("bin").join(java_exe_name()),
    ];
    for c in candidates {
        if c.is_file() {
            // 规范化便于上层比较/去重; 失败时退回原路径。
            return Some(std::fs::canonicalize(&c).unwrap_or(c));
        }
    }
    None
}

/// 下载并安装指定大版本的 JRE, 返回其中 `java` 可执行文件的绝对路径。
///
/// 幂等: 若 `dest_root/jre-{major}` 已存在且能找到 `java`, 直接返回, 不重新下载。
///
/// 失败模式:
///   - 网络/重定向/HTTP 状态错误 → [`CoreError::Network`] / [`CoreError::Download`]。
///   - 解压错误 → [`CoreError::Zip`] (压缩相关统一归到这个变体)。
///   - 解压后找不到 `java` → [`CoreError::JavaNotFound`]。
pub async fn install_jre(
    dl: &crate::download::Downloader,
    dest_root: &Path,
    major: u8,
) -> Result<PathBuf> {
    let target_dir = required_jre_dir(dest_root, major);

    // 1) 幂等短路: 已经装好了就直接用。
    if let Some(java) = find_java_in(&target_dir) {
        return Ok(java);
    }

    // 2) 确保目标父目录存在。
    crate::paths::ensure_dir(dest_root)?;

    let os = adoptium_os();
    let arch = adoptium_arch();
    let url = adoptium_url(major, os, arch);

    // 3) 下载归档字节。Adoptium 会 302 重定向到真正的归档文件; 我们自建一个
    //    *默认跟随重定向* 的 client (reqwest 默认即跟随), 不走 Downloader 以确保
    //    重定向行为可控。windows 是 .zip, mac/linux 是 .tar.gz。
    let archive_bytes = download_following_redirects(dl, &url).await?;

    // 4) 解压到目标目录。先清空旧的半成品目录 (若存在), 保证解压结果干净。
    //    注意: 这里只删我们自己管理的 jre-{major} 目录, 且仅在即将重装时, 不触碰其它路径。
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir).with_path(&target_dir)?;
    }
    crate::paths::ensure_dir(&target_dir)?;

    if os == "windows" {
        extract_zip(&archive_bytes, &target_dir)?;
    } else {
        extract_tar_gz(&archive_bytes, &target_dir)?;
    }

    // 5) 在解压结果里定位 java。
    let java = find_java_in(&target_dir).ok_or(CoreError::JavaNotFound { major })?;

    // 6) (POSIX) 解压 tar 时已保留权限位; zip 不带 unix 权限, 但 windows 的 java.exe
    //    不需要可执行位。这里无需额外 chmod。
    Ok(java)
}

/// 用一个默认跟随重定向的 `reqwest::Client` 下载 URL 的全部字节。
///
/// 复用传入 `Downloader` 的底层 client (同一连接池) 发请求; reqwest 默认策略会跟随
/// 最多 10 次重定向, 正好覆盖 Adoptium 的 302 → 实际归档。HTTP 非 2xx 状态会被
/// `error_for_status` 转成错误。
async fn download_following_redirects(
    dl: &crate::download::Downloader,
    url: &str,
) -> Result<Vec<u8>> {
    let resp = dl
        .client()
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    let bytes = resp.bytes().await?;
    Ok(bytes.to_vec())
}

/// 把 `.tar.gz` 字节解压到 `dest` (mac / linux)。
///
/// 用 `flate2::read::GzDecoder` 解 gzip, 再用 `tar::Archive` 解 tar。`tar` crate 会
/// 保留 entry 的 unix 权限位, 因此解出来的 `bin/java` 仍是可执行的。
fn extract_tar_gz(bytes: &[u8], dest: &Path) -> Result<()> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    // 保留权限 (可执行位); 默认 unpack 即会处理。出错统一归到 Zip 变体 (压缩/归档类)。
    archive
        .unpack(dest)
        .map_err(|e| CoreError::Zip(format!("tar.gz 解压失败: {e}")))?;
    Ok(())
}

/// 把 `.zip` 字节解压到 `dest` (windows)。
///
/// 用 `zip` crate 逐 entry 写出。手动遍历而非 `ZipArchive::extract`, 以便对路径做
/// 防穿越校验 (拒绝 entry 名里的 `..` 逃逸)。
fn extract_zip(bytes: &[u8], dest: &Path) -> Result<()> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| CoreError::Zip(e.to_string()))?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| CoreError::Zip(e.to_string()))?;

        // 用 zip 提供的安全名 (已剥离绝对路径前缀); 仍再做一次 `..` 防穿越。
        let Some(rel) = entry.enclosed_name() else {
            return Err(CoreError::Zip(format!("zip entry 名不安全: {}", entry.name())));
        };
        let out_path = dest.join(&rel);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).with_path(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).with_path(parent)?;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        std::io::Read::read_to_end(&mut entry, &mut buf)
            .map_err(|e| CoreError::Zip(format!("读取 zip entry 失败: {e}")))?;
        std::fs::write(&out_path, buf).with_path(&out_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 纯函数: os / arch 映射 (不依赖运行平台, 直接断言常量集合) ----

    #[test]
    fn adoptium_os_is_a_known_value() {
        // 当前编译平台映射出的 os 必须是 Adoptium 支持的三选一。
        let os = adoptium_os();
        assert!(matches!(os, "mac" | "linux" | "windows"), "unexpected os: {os}");
    }

    #[test]
    fn adoptium_arch_is_a_known_value() {
        let arch = adoptium_arch();
        assert!(matches!(arch, "x64" | "aarch64"), "unexpected arch: {arch}");
    }

    #[test]
    fn arch_maps_x86_64_to_x64_and_arm_to_aarch64() {
        // 与当前编译目标对齐: 验证映射逻辑而非硬编码平台。
        if cfg!(target_arch = "aarch64") {
            assert_eq!(adoptium_arch(), "aarch64");
        } else if cfg!(target_arch = "x86_64") {
            assert_eq!(adoptium_arch(), "x64");
        }
    }

    #[test]
    fn os_maps_each_platform_correctly() {
        if cfg!(target_os = "macos") {
            assert_eq!(adoptium_os(), "mac");
        } else if cfg!(target_os = "windows") {
            assert_eq!(adoptium_os(), "windows");
        } else if cfg!(target_os = "linux") {
            assert_eq!(adoptium_os(), "linux");
        }
    }

    // ---- 纯函数: URL 拼接 (覆盖所有 os/arch 组合, 不联网) ----

    #[test]
    fn url_has_expected_shape() {
        let url = adoptium_url(17, "mac", "aarch64");
        assert_eq!(
            url,
            "https://api.adoptium.net/v3/binary/latest/17/ga/mac/aarch64/jre/hotspot/normal/eclipse"
        );
    }

    #[test]
    fn url_covers_all_combinations() {
        for major in [8u8, 17, 21] {
            for os in ["mac", "linux", "windows"] {
                for arch in ["x64", "aarch64"] {
                    let url = adoptium_url(major, os, arch);
                    // 关键片段都在, 且顺序正确。
                    assert!(url.starts_with("https://api.adoptium.net/v3/binary/latest/"));
                    assert!(url.contains(&format!("/latest/{major}/ga/{os}/{arch}/jre/")));
                    assert!(url.ends_with("/jre/hotspot/normal/eclipse"));
                }
            }
        }
    }

    // ---- required_jre_dir: 纯路径拼接 ----

    #[test]
    fn required_jre_dir_appends_major() {
        let dir = required_jre_dir(Path::new("/data/runtimes"), 21);
        assert_eq!(dir, PathBuf::from("/data/runtimes/jre-21"));
    }

    // ---- find_java_in: 在临时目录里造布局并验证, 测后清理 ----

    /// 造一个唯一的临时目录, 返回路径; 调用方负责清理。
    fn temp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("mc-core-java-install-{tag}-{nanos}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn find_java_in_linux_layout() {
        let root = temp_dir("linux");
        // <root>/jre-21-temurin/bin/java
        let bin = root.join("jre-21-temurin").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let java = bin.join(java_exe_name());
        std::fs::write(&java, b"#!/bin/sh\n").unwrap();

        let found = find_java_in(&root).expect("应能在子目录里找到 java");
        // 规范化后路径的文件名应是 java[.exe]。
        assert_eq!(found.file_name().unwrap(), java_exe_name());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_java_in_macos_layout() {
        let root = temp_dir("macos");
        // <root>/jdk-17.app/Contents/Home/bin/java
        let bin = root
            .join("jdk-17.app")
            .join("Contents")
            .join("Home")
            .join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let java = bin.join(java_exe_name());
        std::fs::write(&java, b"#!/bin/sh\n").unwrap();

        let found = find_java_in(&root).expect("应能识别 macOS Contents/Home 布局");
        assert_eq!(found.file_name().unwrap(), java_exe_name());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_java_in_direct_layout() {
        // java 直接在传入目录的 bin 下 (无套壳子目录)。
        let root = temp_dir("direct");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let java = bin.join(java_exe_name());
        std::fs::write(&java, b"x").unwrap();

        let found = find_java_in(&root).expect("应能在传入目录本身找到 java");
        assert_eq!(found.file_name().unwrap(), java_exe_name());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_java_in_missing_returns_none() {
        let root = temp_dir("empty");
        assert!(find_java_in(&root).is_none());
        std::fs::remove_dir_all(&root).unwrap();

        // 完全不存在的目录也应安静返回 None, 不 panic。
        assert!(find_java_in(Path::new("/definitely/not/here/xyz-123")).is_none());
    }
}
