//! Java 运行时的发现与选择。
//!
//! 这一层负责回答两个问题:
//!   1. 系统上装了哪些 Java? —— [`detect_all`] / [`probe`]
//!   2. 给定的 Minecraft 版本 / version-json 需要哪个大版本, 现有的哪个能用? ——
//!      [`required_major`] + [`select`]
//!
//! 它不下载任何东西 (那是 download/安装层的事), 只负责"看现状 + 做判断"。

pub mod detect;
pub mod install;
pub mod version;

pub use detect::{detect_all, probe};
pub use version::JavaVersion;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 当前平台上 java 可执行文件名 (`java.exe` on Windows, 否则 `java`)。
///
/// detect 与 install 两层都要这同一份"平台 → 可执行名"知识; 统一在此处, 避免两处定义漂移。
pub(crate) fn java_exe_name() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// 一处已发现的 Java 安装。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaInstall {
    /// `java` 可执行文件的 (尽量规范化后的) 绝对路径。
    pub path: PathBuf,
    /// 解析出的版本号。
    pub version: JavaVersion,
    /// 是否 64 位 JVM (取不到位数信息时按 64 位计)。
    pub is_64bit: bool,
    /// 来源描述, 例如 `"PATH"` / `"JAVA_HOME"` / `"system"`。
    pub source: String,
}

/// 计算某个 Minecraft 版本需要的 Java 大版本号。
///
/// 规则 (优先级从高到低):
///   1. 若 version-json 显式给出 `javaVersion.majorVersion` (`profile_major`), 直接用它。
///   2. 否则按 MC 版本号推断:
///        - `<= 1.16`           → Java 8
///        - `1.17 ..= 1.20.4`   → Java 17
///        - `>= 1.20.5`         → Java 21
///   3. 版本号解析失败时, 退回 Java 17 (现代版本里最常见的安全默认值)。
pub fn required_major(mc_version: &str, profile_major: Option<u8>) -> u8 {
    // 1) version-json 的显式要求最权威。
    if let Some(m) = profile_major {
        return m;
    }

    // 2) 解析 MC 版本号 (形如 "1.20.1": 第二段=minor, 第三段=patch)。
    match parse_mc_version(mc_version) {
        Some((minor, patch)) => {
            if minor <= 16 {
                8
            } else if minor < 20 {
                // 1.17 / 1.18 / 1.19 全部用 17。
                17
            } else if minor == 20 {
                // 1.20.0 ..= 1.20.4 → 17; 1.20.5+ → 21。
                if patch <= 4 {
                    17
                } else {
                    21
                }
            } else {
                // 1.21 及以后。
                21
            }
        }
        // 3) 解析失败的安全默认。
        None => 17,
    }
}

/// 解析形如 `"1.20.1"` 的 MC 版本号, 返回 `(minor, patch)`。
///
/// Mojang 的正式版本一律以 `1.` 开头, 真正的"主版本"在第二段。缺失 patch 记为 0。
/// 不符合 `1.x[.y]` 形态 (快照、`inf-`、纯字母等) 时返回 `None`, 交由调用方走默认。
fn parse_mc_version(s: &str) -> Option<(u32, u32)> {
    let mut parts = s.trim().split('.');
    let first = parts.next()?.trim();
    if first != "1" {
        return None;
    }
    // 第二段可能带后缀 (理论上正式版没有, 但稳妥地只取前导数字)。
    let minor: u32 = leading_number(parts.next()?)?;
    let patch: u32 = match parts.next() {
        Some(p) => leading_number(p).unwrap_or(0),
        None => 0,
    };
    Some((minor, patch))
}

/// 取字符串前导的连续数字部分并解析为 `u32`; 无前导数字返回 `None`。
fn leading_number(s: &str) -> Option<u32> {
    let digits: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

/// 从已发现的安装里挑一个满足 `major` 的 Java。
///
/// 只接受**大版本精确匹配**的安装 (启动器对 Java 版本敏感, 跨大版本运行常出问题);
/// 在精确匹配里优先返回 64 位的那个。没有精确匹配时返回 `None`。
pub fn select(installs: &[JavaInstall], major: u8) -> Option<&JavaInstall> {
    installs
        .iter()
        .filter(|i| i.version.major == major)
        // 64 位排在前面: `is_64bit` 取反后 false(=64位) 小于 true(=32位)。
        .min_by_key(|i| !i.is_64bit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn install(major: u8, is_64bit: bool, source: &str) -> JavaInstall {
        JavaInstall {
            path: PathBuf::from(format!("/jvm/{source}/bin/java")),
            version: JavaVersion::new(major, 0, 0),
            is_64bit,
            source: source.to_string(),
        }
    }

    #[test]
    fn profile_major_wins() {
        // 即便 MC 版本号暗示 8, 显式 profile 要求也应覆盖。
        assert_eq!(required_major("1.12.2", Some(21)), 21);
    }

    #[test]
    fn old_versions_need_java8() {
        assert_eq!(required_major("1.7.10", None), 8);
        assert_eq!(required_major("1.12.2", None), 8);
        assert_eq!(required_major("1.16.5", None), 8);
    }

    #[test]
    fn mid_versions_need_java17() {
        assert_eq!(required_major("1.17", None), 17);
        assert_eq!(required_major("1.17.1", None), 17);
        assert_eq!(required_major("1.18.2", None), 17);
        assert_eq!(required_major("1.19.4", None), 17);
        assert_eq!(required_major("1.20", None), 17);
        assert_eq!(required_major("1.20.4", None), 17);
    }

    #[test]
    fn new_versions_need_java21() {
        assert_eq!(required_major("1.20.5", None), 21);
        assert_eq!(required_major("1.20.6", None), 21);
        assert_eq!(required_major("1.21", None), 21);
        assert_eq!(required_major("1.21.4", None), 21);
    }

    #[test]
    fn unparseable_defaults_to_17() {
        assert_eq!(required_major("23w13a", None), 17);
        assert_eq!(required_major("", None), 17);
        assert_eq!(required_major("not-a-version", None), 17);
    }

    #[test]
    fn select_requires_exact_major() {
        let installs = vec![install(8, true, "a"), install(21, true, "b")];
        assert!(select(&installs, 17).is_none());
        assert_eq!(select(&installs, 8).unwrap().source, "a");
        assert_eq!(select(&installs, 21).unwrap().source, "b");
    }

    #[test]
    fn select_prefers_64bit() {
        let installs = vec![install(17, false, "x86"), install(17, true, "x64")];
        let chosen = select(&installs, 17).unwrap();
        assert!(chosen.is_64bit);
        assert_eq!(chosen.source, "x64");
    }

    #[test]
    fn select_returns_only_32bit_when_thats_all() {
        let installs = vec![install(17, false, "x86")];
        let chosen = select(&installs, 17).unwrap();
        assert!(!chosen.is_64bit);
    }

    #[test]
    fn select_empty_is_none() {
        assert!(select(&[], 17).is_none());
    }

    #[test]
    fn java_exe_name_matches_platform() {
        if cfg!(windows) {
            assert_eq!(java_exe_name(), "java.exe");
        } else {
            assert_eq!(java_exe_name(), "java");
        }
    }
}
