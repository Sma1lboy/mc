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
pub fn load_chain<F>(leaf_id: &str, mut loader: F) -> Result<Vec<VersionJson>>
where
    F: FnMut(&str) -> Result<String>,
{
    let mut chain: Vec<VersionJson> = Vec::new();
    let mut current = leaf_id.to_string();
    let mut guard = 0;

    loop {
        guard += 1;
        if guard > 32 {
            return Err(CoreError::other(format!("version inheritance chain too deep at {current}")));
        }
        let raw = loader(&current)?;
        let vj = VersionJson::parse(&raw)
            .map_err(|e| CoreError::Parse { what: format!("version json {current}"), source: e })?;
        let parent = vj.inherits_from.clone();
        chain.push(vj);
        match parent {
            Some(p) => current = p,
            None => break,
        }
    }

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
}
