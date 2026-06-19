//! Merging an `inheritsFrom` chain into a single resolved [`LaunchProfile`].
//! See `docs/modules/version-system.md` §6.

use super::format::{Argument, Arguments, AssetIndexRef, Logging, VersionJson};
use super::library::{Artifact, Library};

/// The flattened result of merging a version json chain (vanilla + loaders).
#[derive(Debug, Clone, Default)]
pub struct LaunchProfile {
    pub id: String,
    pub main_class: String,
    /// Merged libraries, later entries overriding same-coordinate earlier ones.
    pub libraries: Vec<Library>,
    /// 1.13+ structured game arguments (empty when the chain is legacy).
    pub game_args: Vec<Argument>,
    /// 1.13+ structured jvm arguments.
    pub jvm_args: Vec<Argument>,
    /// pre-1.13 single argument string, if the chain used it.
    pub legacy_arguments: Option<String>,
    pub asset_index: Option<AssetIndexRef>,
    pub assets_id: Option<String>,
    pub client_download: Option<Artifact>,
    pub java_major: Option<u8>,
    pub logging: Logging,
}

impl LaunchProfile {
    /// Merge a chain ordered from the base ancestor (index 0) to the leaf (last).
    pub fn from_chain(chain: &[VersionJson]) -> LaunchProfile {
        let mut p = LaunchProfile::default();
        if let Some(leaf) = chain.last() {
            p.id = leaf.id.clone();
        }

        for v in chain {
            if let Some(mc) = &v.main_class {
                p.main_class = mc.clone();
            }
            for lib in &v.libraries {
                upsert_library(&mut p.libraries, lib.clone());
            }
            if let Some(args) = &v.arguments {
                let Arguments { game, jvm } = args.clone();
                p.game_args.extend(game);
                p.jvm_args.extend(jvm);
            }
            if let Some(legacy) = &v.minecraft_arguments {
                p.legacy_arguments = Some(legacy.clone());
            }
            if v.asset_index.is_some() {
                p.asset_index = v.asset_index.clone();
            }
            if v.assets.is_some() {
                p.assets_id = v.assets.clone();
            }
            if v.downloads.client.is_some() {
                p.client_download = v.downloads.client.clone();
            }
            if let Some(jv) = &v.java_version {
                p.java_major = Some(jv.major_version);
            }
            if v.logging.client.is_some() {
                p.logging = v.logging.clone();
            }
        }

        p
    }
}

/// Insert a library, replacing any existing one with the same `group:artifact`
/// (keeping its position) so loader overrides win without duplicating.
fn upsert_library(libs: &mut Vec<Library>, lib: Library) {
    let key = library_key(&lib);
    if let Some(slot) = libs.iter_mut().find(|l| library_key(l) == key && key.is_some()) {
        *slot = lib;
    } else {
        libs.push(lib);
    }
}

fn library_key(lib: &Library) -> Option<String> {
    // Key on group:artifact:classifier — NOT version — so loader overrides (same
    // coordinate, newer version) collapse, while platform natives (same
    // group:artifact:version but a distinct `natives-*` classifier) coexist with
    // the main jar instead of clobbering it.
    lib.spec().map(|s| {
        format!("{}:{}:{}", s.group, s.artifact, s.classifier.unwrap_or_default())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vj(s: &str) -> VersionJson {
        VersionJson::parse(s).unwrap()
    }

    #[test]
    fn leaf_main_class_overrides() {
        let base = vj(r#"{"id":"1.20.1","mainClass":"net.minecraft.client.main.Main","libraries":[]}"#);
        let loader = vj(r#"{"id":"fabric","inheritsFrom":"1.20.1","mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient","libraries":[]}"#);
        let p = LaunchProfile::from_chain(&[base, loader]);
        assert_eq!(p.main_class, "net.fabricmc.loader.impl.launch.knot.KnotClient");
        assert_eq!(p.id, "fabric");
    }

    #[test]
    fn libraries_dedupe_by_coordinate() {
        let base = vj(r#"{"id":"a","libraries":[{"name":"com.x:y:1.0"}]}"#);
        let child = vj(r#"{"id":"b","inheritsFrom":"a","libraries":[{"name":"com.x:y:2.0"}]}"#);
        let p = LaunchProfile::from_chain(&[base, child]);
        assert_eq!(p.libraries.len(), 1);
        assert_eq!(p.libraries[0].name, "com.x:y:2.0");
    }

    #[test]
    fn inherits_asset_index_from_base() {
        let base = vj(r#"{"id":"a","assets":"5","assetIndex":{"id":"5","url":"http://x"},"libraries":[]}"#);
        let child = vj(r#"{"id":"b","inheritsFrom":"a","libraries":[]}"#);
        let p = LaunchProfile::from_chain(&[base, child]);
        assert_eq!(p.assets_id.as_deref(), Some("5"));
        assert!(p.asset_index.is_some());
    }
}
