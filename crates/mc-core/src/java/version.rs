//! Java 版本号的解析与比较。
//!
//! Java 的版本字符串历史上有两种写法:
//!   - 旧版 (Java 8 及更早): `1.8.0_391` —— 真正的主版本藏在第二段, `1.8` 即 major=8。
//!   - 新版 (Java 9+):       `17.0.14` / `21.0.2` —— 第一段就是 major。
//!
//! `java -version` 把版本信息打印在 **stderr**, 形如:
//! ```text
//! openjdk version "17.0.14" 2025-01-21
//! OpenJDK Runtime Environment Temurin-17.0.14+7 (build 17.0.14+7)
//! OpenJDK 64-Bit Server VM Temurin-17.0.14+7 (build 17.0.14+7, mixed mode)
//! ```

use serde::{Deserialize, Serialize};

/// 一个规范化后的 Java 版本号 (major.minor.patch)。
///
/// `major` 始终是"市场版本号" (8、17、21…), 而非旧式的 `1.x`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaVersion {
    pub major: u8,
    pub minor: u16,
    pub patch: u16,
}

impl JavaVersion {
    /// 直接构造 (主要给测试/已知版本用)。
    pub fn new(major: u8, minor: u16, patch: u16) -> Self {
        JavaVersion { major, minor, patch }
    }

    /// 从 `java -version` 的 (stderr) 文本里解析出版本号。
    ///
    /// 兼容 `version "1.8.0_391"` 与 `version "17.0.14"` 两种格式; 找不到带引号
    /// 的版本字面量时返回 `None`。
    pub fn parse_from_output(s: &str) -> Option<JavaVersion> {
        // 取第一个出现在双引号里的版本字面量, 例如 `"17.0.14"` 或 `"1.8.0_391"`。
        // 不依赖前缀关键字 (openjdk/java/...), 因为不同发行版措辞不一。
        let literal = first_quoted(s)?;
        Self::parse_literal(literal)
    }

    /// 解析裸版本字面量, 不含引号, 例如 `1.8.0_391` 或 `17.0.14`。
    fn parse_literal(literal: &str) -> Option<JavaVersion> {
        // Java 8 风格的 `_build` 后缀对 major/minor/patch 无意义, 截掉。
        // 同理一些 EA 版本带 `-ea` / `+7` 之类后缀, 一并去掉。
        let core = literal
            .split(|c| c == '_' || c == '-' || c == '+')
            .next()
            .unwrap_or(literal);

        let mut nums = core.split('.');
        let first: u32 = nums.next()?.trim().parse().ok()?;

        if first == 1 {
            // 旧式 `1.X.Y`: 真正的 major 在第二段, patch 在第三段。
            let major: u8 = nums.next()?.trim().parse().ok()?;
            let patch: u16 = nums.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
            // 旧式没有独立的 minor 概念, 统一记为 0。
            Some(JavaVersion { major, minor: 0, patch })
        } else {
            // 新式 `X.Y.Z`: 直接映射。
            let major: u8 = u8::try_from(first).ok()?;
            let minor: u16 = nums.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
            let patch: u16 = nums.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
            Some(JavaVersion { major, minor, patch })
        }
    }

    /// 该 Java 是否至少为 `major` 大版本。
    pub fn is_at_least(&self, major: u8) -> bool {
        self.major >= major
    }
}

/// 取字符串中第一个被双引号包裹的内容。
fn first_quoted(s: &str) -> Option<&str> {
    let start = s.find('"')? + 1;
    let rest = &s[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

impl PartialOrd for JavaVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JavaVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // 按 (major, minor, patch) 字典序比较。
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
    }
}

impl std::fmt::Display for JavaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_java8_legacy() {
        let out = r#"openjdk version "1.8.0_391"
OpenJDK Runtime Environment (build 1.8.0_391-b13)
OpenJDK 64-Bit Server VM (build 25.391-b13, mixed mode)"#;
        let v = JavaVersion::parse_from_output(out).unwrap();
        assert_eq!(v, JavaVersion::new(8, 0, 0));
        assert!(v.is_at_least(8));
        assert!(!v.is_at_least(17));
    }

    #[test]
    fn parses_java17() {
        let out = r#"openjdk version "17.0.14" 2025-01-21
OpenJDK Runtime Environment Temurin-17.0.14+7 (build 17.0.14+7)
OpenJDK 64-Bit Server VM Temurin-17.0.14+7 (build 17.0.14+7, mixed mode)"#;
        let v = JavaVersion::parse_from_output(out).unwrap();
        assert_eq!(v, JavaVersion::new(17, 0, 14));
        assert!(v.is_at_least(17));
        assert!(!v.is_at_least(21));
    }

    #[test]
    fn parses_java21() {
        let out = r#"openjdk version "21.0.2" 2024-01-16 LTS"#;
        let v = JavaVersion::parse_from_output(out).unwrap();
        assert_eq!(v, JavaVersion::new(21, 0, 2));
        assert!(v.is_at_least(21));
    }

    #[test]
    fn parses_oracle_java8_no_build() {
        // 没有 `_build` 后缀也要能解析。
        let v = JavaVersion::parse_from_output(r#"java version "1.8.0""#).unwrap();
        assert_eq!(v, JavaVersion::new(8, 0, 0));
    }

    #[test]
    fn parses_ea_suffix() {
        let v = JavaVersion::parse_from_output(r#"openjdk version "22-ea" 2024-03-19"#).unwrap();
        assert_eq!(v.major, 22);
    }

    #[test]
    fn ordering_is_by_components() {
        assert!(JavaVersion::new(8, 0, 0) < JavaVersion::new(17, 0, 0));
        assert!(JavaVersion::new(17, 0, 14) > JavaVersion::new(17, 0, 2));
        assert!(JavaVersion::new(21, 0, 0) > JavaVersion::new(17, 9, 99));
        assert_eq!(JavaVersion::new(17, 0, 1), JavaVersion::new(17, 0, 1));
    }

    #[test]
    fn display_roundtrips_shape() {
        assert_eq!(JavaVersion::new(17, 0, 14).to_string(), "17.0.14");
        assert_eq!(JavaVersion::new(8, 0, 0).to_string(), "8.0.0");
    }

    #[test]
    fn returns_none_without_quoted_version() {
        assert!(JavaVersion::parse_from_output("no version here").is_none());
        assert!(JavaVersion::parse_from_output("").is_none());
    }
}
