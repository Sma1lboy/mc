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
                // Lightweight prefix/substring match instead of full regex to avoid
                // a regex dependency; Mojang's os.version patterns are simple.
                if !ctx.os_version.is_empty() && !ctx.os_version.contains(version.trim_start_matches('^').trim_end_matches(".*").trim_end_matches('$')) {
                    return false;
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
