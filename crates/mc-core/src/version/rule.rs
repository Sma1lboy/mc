//! Mojang rule evaluation for libraries and arguments. See
//! `docs/modules/version-system.md` §4.

use std::collections::BTreeMap;

use serde::Deserialize;

use mc_types::{Arch, Os, Platform};

/// The set of feature toggles a launch can enable (demo mode, custom resolution…).
/// Used to evaluate game-argument rules.
#[derive(Debug, Clone, Default)]
pub struct FeatureSet {
    pub is_demo_user: bool,
    pub has_custom_resolution: bool,
    pub has_quick_plays_support: bool,
    pub is_quick_play_singleplayer: bool,
    pub is_quick_play_multiplayer: bool,
    pub is_quick_play_realms: bool,
}

impl FeatureSet {
    fn get(&self, name: &str) -> bool {
        match name {
            "is_demo_user" => self.is_demo_user,
            "has_custom_resolution" => self.has_custom_resolution,
            "has_quick_plays_support" => self.has_quick_plays_support,
            "is_quick_play_singleplayer" => self.is_quick_play_singleplayer,
            "is_quick_play_multiplayer" => self.is_quick_play_multiplayer,
            "is_quick_play_realms" => self.is_quick_play_realms,
            _ => false,
        }
    }
}

/// Everything a rule can match against.
#[derive(Debug, Clone)]
pub struct RuntimeContext {
    pub platform: Platform,
    /// OS version string, matched against the `os.version` regex when present.
    pub os_version: String,
    pub features: FeatureSet,
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self { platform: Platform::current(), os_version: String::new(), features: FeatureSet::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OsConstraint {
    pub name: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub action: Action,
    #[serde(default)]
    pub os: Option<OsConstraint>,
    /// Feature constraints, e.g. `{ "is_demo_user": true }`.
    #[serde(default)]
    pub features: Option<BTreeMap<String, bool>>,
}

impl Rule {
    /// Does this single rule match the context (ignoring its action)?
    fn matches(&self, ctx: &RuntimeContext) -> bool {
        if let Some(os) = &self.os {
            if let Some(name) = &os.name {
                if name != ctx.platform.os.mojang_name() {
                    return false;
                }
            }
            if let Some(arch) = &os.arch {
                if !arch_matches(arch, ctx.platform.arch) {
                    return false;
                }
            }
            if let Some(version) = &os.version {
                // Lightweight prefix match instead of a full regex (no regex dep;
                // Mojang's os.version patterns are simple, `^`-anchored, escaped-dot).
                // Only consulted once os_version is populated — it is empty today, so
                // this branch is inert at runtime (see ADR-0001).
                if !ctx.os_version.is_empty() {
                    let prefix = os_version_prefix(version);
                    if !prefix.is_empty() && !ctx.os_version.starts_with(&prefix) {
                        return false;
                    }
                }
            }
        }
        if let Some(features) = &self.features {
            for (name, expected) in features {
                if ctx.features.get(name) != *expected {
                    return false;
                }
            }
        }
        true
    }
}

/// 把 Mojang 的简单 os.version 正则化简成可前缀匹配的纯文本前缀。
///
/// Mojang 的模式形态很窄:`^` 锚点 + 转义点(`\.`)+ 可选 `.*` / `\d+` 尾部通配,
/// 例如 `^10\.`、`^6\.2`、`^10\.0\.`。这里去掉锚点与尾部通配、把 `\.` 反转义成 `.`,
/// 得到一个用于 `starts_with` 的前缀。**先前的实现忘了反转义 `\.`**,导致即便
/// 填了 os_version 也永远匹配不上;此函数修正之(并加测试覆盖)。
fn os_version_prefix(pattern: &str) -> String {
    let mut s = pattern.trim();
    s = s.strip_prefix('^').unwrap_or(s);
    s = s.strip_suffix('$').unwrap_or(s);
    for suffix in ["\\d+", "\\d*", ".*", ".+"] {
        s = s.strip_suffix(suffix).unwrap_or(s);
    }
    s.replace("\\.", ".").trim_end_matches('\\').to_string()
}

fn arch_matches(spec: &str, arch: Arch) -> bool {
    match spec {
        "x86" => arch == Arch::X86,
        "x64" | "x86_64" => arch == Arch::X64,
        "arm64" | "aarch64" => arch == Arch::Arm64,
        "arm" | "arm32" => arch == Arch::Arm32,
        _ => false,
    }
}

/// Evaluate a rule list. If empty, the item is allowed. Otherwise the default is
/// "disallow" and the last matching rule's action wins.
pub fn rules_allow(rules: &[Rule], ctx: &RuntimeContext) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule.matches(ctx) {
            allowed = rule.action == Action::Allow;
        }
    }
    allowed
}

/// Convenience for matching against the current OS only (used by old-style native maps).
pub fn os_key(os: Os) -> &'static str {
    os.mojang_name()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(os: Os, arch: Arch) -> RuntimeContext {
        RuntimeContext {
            platform: Platform { os, arch },
            os_version: String::new(),
            features: FeatureSet::default(),
        }
    }

    #[test]
    fn empty_rules_allow() {
        assert!(rules_allow(&[], &ctx(Os::Linux, Arch::X64)));
    }

    #[test]
    fn allow_only_windows() {
        let rules: Vec<Rule> = serde_json::from_str(
            r#"[{"action":"allow","os":{"name":"windows"}}]"#,
        )
        .unwrap();
        assert!(rules_allow(&rules, &ctx(Os::Windows, Arch::X64)));
        assert!(!rules_allow(&rules, &ctx(Os::Linux, Arch::X64)));
    }

    #[test]
    fn allow_all_except_osx() {
        let rules: Vec<Rule> = serde_json::from_str(
            r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#,
        )
        .unwrap();
        assert!(rules_allow(&rules, &ctx(Os::Linux, Arch::X64)));
        assert!(!rules_allow(&rules, &ctx(Os::MacOs, Arch::Arm64)));
    }

    #[test]
    fn os_version_prefix_unescapes_and_strips() {
        assert_eq!(os_version_prefix(r"^10\."), "10.");
        assert_eq!(os_version_prefix(r"^6\.2"), "6.2");
        assert_eq!(os_version_prefix(r"^10\.0\."), "10.0.");
        assert_eq!(os_version_prefix(r"^10\.\d+$"), "10.");
    }

    #[test]
    fn os_version_rule_matches_only_when_prefix_agrees() {
        // JSON 里点是双反斜杠转义;serde 解出来是 `^10\.`。
        let rules: Vec<Rule> = serde_json::from_str(
            r#"[{"action":"allow","os":{"name":"windows","version":"^10\\."}}]"#,
        )
        .unwrap();
        let mut c = ctx(Os::Windows, Arch::X64);
        c.os_version = "10.0.19045".into();
        assert!(rules_allow(&rules, &c)); // Win10 → 命中(旧实现因没反转义会误判为不命中)
        c.os_version = "6.2.9200".into();
        assert!(!rules_allow(&rules, &c)); // Win8.1 → 不命中 → 默认 disallow
    }

    #[test]
    fn empty_os_version_ignores_the_version_clause() {
        // os_version 为空(运行时现状)时跳过 version 子句,仅按 name 匹配 —— 行为不变。
        let rules: Vec<Rule> = serde_json::from_str(
            r#"[{"action":"allow","os":{"name":"windows","version":"^10\\."}}]"#,
        )
        .unwrap();
        assert!(rules_allow(&rules, &ctx(Os::Windows, Arch::X64)));
    }

    #[test]
    fn feature_rule() {
        let rules: Vec<Rule> = serde_json::from_str(
            r#"[{"action":"allow","features":{"is_demo_user":true}}]"#,
        )
        .unwrap();
        let mut c = ctx(Os::Linux, Arch::X64);
        assert!(!rules_allow(&rules, &c));
        c.features.is_demo_user = true;
        assert!(rules_allow(&rules, &c));
    }
}
