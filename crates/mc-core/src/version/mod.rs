//! Version & component system: parse Mojang/loader version json, evaluate rules,
//! resolve Gradle coordinates, and merge an `inheritsFrom` chain into one
//! [`LaunchProfile`].

pub mod format;
pub mod gradle;
pub mod library;
pub mod pack;
pub mod profile;
pub mod rule;

pub use format::{Argument, Arguments, AssetIndexRef, StringOrList, VersionJson};
pub use gradle::GradleSpec;
pub use library::{
    classpath_libraries, select_native_libraries, Artifact, Library, ResolvedFile,
};
pub use pack::{
    known_loader, loader_uid, Component, KnownLoader, PackProfile, Require, KNOWN_LOADERS,
    PACK_FILE, UID_FABRIC_INTERMEDIARY, UID_MINECRAFT, UID_QUILT_HASHED,
};
pub use profile::LaunchProfile;
pub use rule::{rules_allow, FeatureSet, Rule, RuntimeContext};

use crate::error::{CoreError, Result};

/// Default Maven repository for `url`-only / coordinate-only libraries.
pub const DEFAULT_LIBRARIES_MAVEN: &str = "https://libraries.minecraft.net";

/// Load and merge a version's full `inheritsFrom` chain using `loader` to fetch
/// each version json (by id) as a string. Returns the chain ordered base→leaf.
///
/// `loader` is supplied by the caller so this stays IO-free and testable; the
/// instance layer passes a closure that reads `versions/<id>/<id>.json`.
/// 一个 `inheritsFrom` 链节点:该层的载荷 + 它的父 id(`None` = 已到根)。
pub struct InheritNode<T> {
    pub payload: T,
    pub parent: Option<String>,
}

/// 沿 `inheritsFrom` 链从 `leaf_id` 走到根,逐层用 `read_node` 取「载荷 + 父 id」,
/// 返回 **leaf→base** 顺序的载荷序列。
///
/// 这是 inheritsFrom 遍历的**唯一**实现:守护逻辑(环检测 + 深度上限)集中于此,
/// 调用方只通过 `read_node` 决定每层读什么(完整 version json / 轻量 head)以及
/// 如何容错(严格 `?` 传播,或读失败时返回 `parent: None` 优雅停在当前层)。
pub fn walk_inherits<T, F>(leaf_id: &str, mut read_node: F) -> Result<Vec<T>>
where
    F: FnMut(&str) -> Result<InheritNode<T>>,
{
    let mut out: Vec<T> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut current = leaf_id.to_string();

    loop {
        if !seen.insert(current.clone()) {
            return Err(CoreError::other(format!("version inheritance cycle at {current}")));
        }
        if seen.len() > 64 {
            return Err(CoreError::other(format!(
                "version inheritance chain too deep at {current}"
            )));
        }
        let node = read_node(&current)?;
        let parent = node.parent.clone();
        out.push(node.payload);
        match parent {
            Some(p) => current = p,
            None => break,
        }
    }

    Ok(out)
}

/// Thin, strict adapter over [`walk_inherits`]: fetch each version json via
/// `loader`, parse it (errors propagate), and return the chain base→leaf.
pub fn load_chain<F>(leaf_id: &str, mut loader: F) -> Result<Vec<VersionJson>>
where
    F: FnMut(&str) -> Result<String>,
{
    let mut chain = walk_inherits(leaf_id, |id| {
        let raw = loader(id)?;
        let vj = VersionJson::parse(&raw)
            .map_err(|e| CoreError::Parse { what: format!("version json {id}"), source: e })?;
        let parent = vj.inherits_from.clone();
        Ok(InheritNode { payload: vj, parent })
    })?;

    // `chain` is leaf→base; reverse to base→leaf for merging.
    chain.reverse();
    Ok(chain)
}

/// Convenience: load the chain and merge it into a [`LaunchProfile`].
pub fn resolve_profile<F>(leaf_id: &str, loader: F) -> Result<LaunchProfile>
where
    F: FnMut(&str) -> Result<String>,
{
    let chain = load_chain(leaf_id, loader)?;
    Ok(LaunchProfile::from_chain(&chain))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn resolves_two_level_chain() {
        let mut store = HashMap::new();
        store.insert(
            "1.20.1".to_string(),
            r#"{"id":"1.20.1","mainClass":"M","assets":"5","libraries":[]}"#.to_string(),
        );
        store.insert(
            "fabric-1.20.1".to_string(),
            r#"{"id":"fabric-1.20.1","inheritsFrom":"1.20.1","mainClass":"K","libraries":[]}"#.to_string(),
        );

        let profile = resolve_profile("fabric-1.20.1", |id| {
            store.get(id).cloned().ok_or_else(|| CoreError::VersionNotFound(id.to_string()))
        })
        .unwrap();

        assert_eq!(profile.main_class, "K");
        assert_eq!(profile.assets_id.as_deref(), Some("5"));
    }

    #[test]
    fn walk_inherits_yields_leaf_to_base_in_order() {
        // 纯遍历(无 IO):用父映射验证顺序与泛型载荷。
        let parents: HashMap<&str, Option<&str>> =
            HashMap::from([("leaf", Some("mid")), ("mid", Some("base")), ("base", None)]);
        let ids = walk_inherits("leaf", |id| {
            Ok::<_, CoreError>(InheritNode {
                payload: id.to_string(),
                parent: parents.get(id).and_then(|p| p.map(str::to_string)),
            })
        })
        .unwrap();
        assert_eq!(ids, vec!["leaf", "mid", "base"]);
    }

    #[test]
    fn walk_inherits_detects_cycle_instead_of_looping() {
        // a → b → a:必须报环错误,而不是无限循环。这是统一遍历后两个调用方都获得的护栏。
        let parents: HashMap<&str, Option<&str>> =
            HashMap::from([("a", Some("b")), ("b", Some("a"))]);
        let err = walk_inherits("a", |id| {
            Ok::<_, CoreError>(InheritNode {
                payload: id.to_string(),
                parent: parents.get(id).and_then(|p| p.map(str::to_string)),
            })
        })
        .unwrap_err();
        assert!(matches!(err, CoreError::Other(_)));
    }

    #[test]
    fn load_chain_detects_cycle() {
        // load_chain 作为薄适配器也继承环检测(经 walk_inherits)。
        let store = HashMap::from([
            ("a".to_string(), r#"{"id":"a","inheritsFrom":"b","libraries":[]}"#.to_string()),
            ("b".to_string(), r#"{"id":"b","inheritsFrom":"a","libraries":[]}"#.to_string()),
        ]);
        let err = load_chain("a", |id| {
            store.get(id).cloned().ok_or_else(|| CoreError::VersionNotFound(id.to_string()))
        })
        .unwrap_err();
        assert!(matches!(err, CoreError::Other(_)));
    }
}
