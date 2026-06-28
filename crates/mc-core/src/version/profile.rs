//! Merging an `inheritsFrom` chain into a single resolved [`LaunchProfile`].
//! See `docs/modules/version-system.md` §6.

use super::format::{Argument, Arguments, AssetIndexRef, Logging, VersionJson};
use super::library::{Artifact, Library};
use super::pack::Component;

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
    /// Id of the chain member that actually declared `downloads.client` (the
    /// vanilla base). The client jar lives at `versions/<this>/<this>.jar`, which
    /// is **not** necessarily the leaf `id` — modpack/loader instances are thin
    /// stubs whose own dir holds no jar. Used to key the classpath + download.
    pub client_jar_id: Option<String>,
    pub java_major: Option<u8>,
    pub logging: Logging,
}

impl LaunchProfile {
    /// Merge a chain ordered from the base ancestor (index 0) to the leaf (last).
    ///
    /// 排序来源是父指针(`inheritsFrom`):base→leaf。合并数学(库 upsert、mainClass
    /// 末者胜、args 追加)与 [`LaunchProfile::from_components`] 完全共享 [`merge_one`]。
    pub fn from_chain(chain: &[VersionJson]) -> LaunchProfile {
        let mut p = LaunchProfile::default();
        if let Some(leaf) = chain.last() {
            p.id = leaf.id.clone();
        }
        for v in chain {
            merge_one(&mut p, v);
        }
        p
    }

    /// The id of the chain member whose `versions/<id>/` dir holds the Minecraft
    /// jar: the member that declared `downloads.client` (the vanilla base), or the
    /// leaf [`id`](Self::id) for a self-contained vanilla instance. The download
    /// path ([`crate::meta::client_jar_item`]) and the classpath entry
    /// ([`crate::launch::build_classpath`]) MUST agree on this id; routing both
    /// through this one accessor is what keeps the "key a chain-inherited resource
    /// by the leaf id" bug from resurfacing (the two used to hand-mirror the
    /// `unwrap_or(&id)` fallback).
    pub fn client_jar_id(&self) -> &str {
        self.client_jar_id.as_deref().unwrap_or(&self.id)
    }

    /// 按**显式组件列表顺序**合并(MultiMC/Prism 组件模型),复用与 [`from_chain`]
    /// 完全相同的合并逻辑([`merge_one`]),只把排序来源从父指针换成列表顺序。
    ///
    /// 见 `docs/modules/instance-and-components.md` §2.5。每项为 `(组件, 它解析到的
    /// 版本文件)`,顺序即合并优先级(后盖前)。`disabled` 的组件被跳过(参与列表但
    /// 不参与合并)。`id` 取**最后一个 active 组件**的版本文件 id(末者胜,即 leaf)。
    ///
    /// IO 像 `load_chain` 那样由调用方注入:调用方按列表顺序为每个组件读出其版本文件
    /// (`patches/<uid>.json` 覆盖或 meta 缓存),再传入这里做纯合并。
    pub fn from_components(components: &[(Component, VersionJson)]) -> LaunchProfile {
        let mut p = LaunchProfile::default();
        for (comp, vj) in components {
            if !comp.is_active() {
                continue;
            }
            // id 取最后一个参与合并的组件(末者胜),与 from_chain 的 leaf 语义一致。
            p.id = vj.id.clone();
            merge_one(&mut p, vj);
        }
        p
    }
}

/// 把单个版本文件合并进 `p`(库 upsert、mainClass/assetIndex/client/java/logging 末者胜、
/// args 追加)。`from_chain` 与 `from_components` 共用,保证两种排序来源得到同样的合并
/// 数学。
fn merge_one(p: &mut LaunchProfile, v: &VersionJson) {
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
        // The jar belongs to *this* version's dir, not the leaf's. (末者胜:若多层
        // 都声明 client,取最靠近 leaf 的那层,与 client_download 保持一致。)
        p.client_jar_id = Some(v.id.clone());
    }
    if let Some(jv) = &v.java_version {
        p.java_major = Some(jv.major_version);
    }
    if v.logging.client.is_some() {
        p.logging = v.logging.clone();
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
    fn client_jar_id_is_base_not_leaf() {
        // 三层薄存根链:实例存根 → fabric-loader → 原版(后者独有 downloads.client)。
        // client jar 应归属原版那层,而非 leaf 存根(其目录里根本没有 jar)。
        let vanilla = vj(
            r#"{"id":"26.2","mainClass":"net.minecraft.client.main.Main","libraries":[],
                "downloads":{"client":{"url":"https://cdn/c.jar","sha1":"abc","size":10}}}"#,
        );
        let loader = vj(r#"{"id":"fabric-loader-0.19.3-26.2","inheritsFrom":"26.2","libraries":[]}"#);
        let stub = vj(r#"{"id":"Fabulously Optimized","inheritsFrom":"fabric-loader-0.19.3-26.2"}"#);
        let p = LaunchProfile::from_chain(&[vanilla, loader, stub]);
        assert_eq!(p.id, "Fabulously Optimized");
        assert_eq!(p.client_jar_id.as_deref(), Some("26.2"));
        assert!(p.client_download.is_some());
        // The accessor both the classpath read and the download write go through
        // resolves to the base, NOT the leaf stub whose dir holds no jar.
        assert_eq!(p.client_jar_id(), "26.2");

        // When no chain member declared downloads.client the field stays None and
        // the accessor falls back to the leaf id (exercises `unwrap_or(&self.id)`).
        let no_client = LaunchProfile::from_chain(&[vj(r#"{"id":"1.20.1","libraries":[]}"#)]);
        assert!(no_client.client_jar_id.is_none());
        assert_eq!(no_client.client_jar_id(), "1.20.1");
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

    // ---- from_components:与 from_chain 同样的合并数学,排序来源换成显式列表 ----

    #[test]
    fn from_components_merges_in_list_order() {
        // vanilla → intermediary(夹中间)→ fabric-loader,顺序即合并序。
        let vanilla = Component::important(super::super::pack::UID_MINECRAFT, Some("1.20.1".into()));
        let inter = Component::dependency(super::super::pack::UID_FABRIC_INTERMEDIARY, Some("1.20.1".into()));
        let loader = Component::important("net.fabricmc.fabric-loader", Some("0.15.7".into()));

        let comps = vec![
            (vanilla, vj(r#"{"id":"1.20.1","mainClass":"net.minecraft.client.main.Main","assets":"5","libraries":[{"name":"com.x:y:1.0"}]}"#)),
            (inter, vj(r#"{"id":"intermediary","libraries":[{"name":"net.fabricmc:intermediary:1.20.1"}]}"#)),
            (loader, vj(r#"{"id":"fabric-loader-0.15.7","mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient","libraries":[{"name":"com.x:y:2.0"}]}"#)),
        ];

        let p = LaunchProfile::from_components(&comps);
        // mainClass 末者胜(loader 覆盖 vanilla)。
        assert_eq!(p.main_class, "net.fabricmc.loader.impl.launch.knot.KnotClient");
        // assets 来自 vanilla(loader 未覆盖)。
        assert_eq!(p.assets_id.as_deref(), Some("5"));
        // 库 upsert:com.x:y 被 loader 的 2.0 覆盖,intermediary 库新增。
        assert!(p.libraries.iter().any(|l| l.name == "com.x:y:2.0"));
        assert!(!p.libraries.iter().any(|l| l.name == "com.x:y:1.0"));
        assert!(p.libraries.iter().any(|l| l.name == "net.fabricmc:intermediary:1.20.1"));
        // id 取最后一个 active 组件(loader)的版本文件 id。
        assert_eq!(p.id, "fabric-loader-0.15.7");
    }

    #[test]
    fn from_components_skips_disabled() {
        let vanilla = Component::important(super::super::pack::UID_MINECRAFT, Some("1.20.1".into()));
        let mut loader = Component::important("net.fabricmc.fabric-loader", Some("0.15.7".into()));
        loader.disabled = true; // 禁用的 loader 不参与合并。

        let comps = vec![
            (vanilla, vj(r#"{"id":"1.20.1","mainClass":"VANILLA","libraries":[]}"#)),
            (loader, vj(r#"{"id":"fabric","mainClass":"LOADER","libraries":[]}"#)),
        ];
        let p = LaunchProfile::from_components(&comps);
        // loader 被跳过 → mainClass 仍是 vanilla 的;id 也是 vanilla 的。
        assert_eq!(p.main_class, "VANILLA");
        assert_eq!(p.id, "1.20.1");
    }

    #[test]
    fn from_components_matches_from_chain_for_equivalent_order() {
        // 同样两层,from_components(列表序)应与 from_chain(父指针)得到一致结果。
        let base_json = r#"{"id":"1.20.1","mainClass":"M","assets":"5","libraries":[{"name":"a:b:1.0"}]}"#;
        let leaf_json = r#"{"id":"loader","inheritsFrom":"1.20.1","mainClass":"K","libraries":[{"name":"a:b:2.0"}]}"#;

        let chain = LaunchProfile::from_chain(&[vj(base_json), vj(leaf_json)]);

        let comps = vec![
            (Component::important(super::super::pack::UID_MINECRAFT, Some("1.20.1".into())), vj(base_json)),
            (Component::important("net.fabricmc.fabric-loader", Some("0.15.7".into())), vj(leaf_json)),
        ];
        let components = LaunchProfile::from_components(&comps);

        assert_eq!(chain.main_class, components.main_class);
        assert_eq!(chain.id, components.id);
        assert_eq!(chain.libraries.len(), components.libraries.len());
        assert_eq!(chain.libraries[0].name, components.libraries[0].name);
        assert_eq!(chain.assets_id, components.assets_id);
    }
}
