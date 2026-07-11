    use super::*;

    // ---- serde round-trip ----

    #[test]
    fn pack_profile_roundtrip_with_serde_renames() {
        let mut pack = PackProfile::new();
        pack.components.push(Component::important(UID_MINECRAFT, Some("1.20.1".into())));
        let mut loader = Component::important("net.fabricmc.fabric-loader", Some("0.15.7".into()));
        loader.cached_name = Some("Fabric Loader".into());
        loader.cached_requires = vec![Require::equals(UID_FABRIC_INTERMEDIARY, "1.20.1")];
        pack.components.push(loader);
        let mut inter = Component::dependency(UID_FABRIC_INTERMEDIARY, Some("1.20.1".into()));
        inter.cached_version = Some("1.20.1".into());
        pack.components.push(inter);

        let json = serde_json::to_string_pretty(&pack).unwrap();
        // 验证 JSON 键名走的是 camelCase / Prism 命名,而非 Rust 字段名。
        assert!(json.contains("\"formatVersion\""));
        assert!(json.contains("\"dependencyOnly\""));
        assert!(json.contains("\"cachedVolatile\""));
        assert!(json.contains("\"cachedRequires\""));
        assert!(json.contains("\"cachedName\""));
        // Require 的 equals 键名是 "equals" 而不是 "equals_version"。
        assert!(json.contains("\"equals\""));
        // 默认 false 的布尔不应出现(disabled 未置位)。
        assert!(!json.contains("\"disabled\""));

        let back: PackProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pack);
    }

    #[test]
    fn volatile_alias_reads_both_keys_writes_cached_volatile() {
        // Prism bug-for-bug:输入 "volatile" 应被接受。
        let raw = r#"{
            "formatVersion": 1,
            "components": [
                {"uid":"org.lwjgl3","version":"3.3.1","dependencyOnly":true,"volatile":true}
            ]
        }"#;
        let pack: PackProfile = serde_json::from_str(raw).unwrap();
        assert!(pack.components[0].cached_volatile);
        assert!(pack.components[0].dependency_only);
        // 输出统一写 cachedVolatile。
        let json = serde_json::to_string(&pack).unwrap();
        assert!(json.contains("\"cachedVolatile\":true"));
        assert!(!json.contains("\"volatile\":"));
    }

    #[test]
    fn parses_minimal_pack() {
        let raw = r#"{"formatVersion":1,"components":[{"uid":"net.minecraft","version":"1.21","important":true}]}"#;
        let pack: PackProfile = serde_json::from_str(raw).unwrap();
        assert_eq!(pack.format_version, 1);
        assert_eq!(pack.components.len(), 1);
        assert_eq!(pack.minecraft_version(), Some("1.21"));
        assert!(pack.components[0].important);
    }

    // ---- set_component / get_component ----

    #[test]
    fn set_component_appends_then_updates() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        assert_eq!(pack.components.len(), 1);
        assert_eq!(pack.minecraft_version(), Some("1.20.1"));
        assert!(pack.get_component(UID_MINECRAFT).unwrap().important);

        // 再次 set 同 uid → 就地更新版本,不新增。
        pack.set_component(UID_MINECRAFT, "1.20.4", false);
        assert_eq!(pack.components.len(), 1);
        assert_eq!(pack.minecraft_version(), Some("1.20.4"));
        // important 已置位,不应被 false 复位。
        assert!(pack.get_component(UID_MINECRAFT).unwrap().important);

        // 不同 uid → 追加。
        pack.set_component("net.minecraftforge", "47.2.0", true);
        assert_eq!(pack.components.len(), 2);
        assert_eq!(pack.get_component("net.minecraftforge").unwrap().version.as_deref(), Some("47.2.0"));
    }

    // ---- detect_loader ----

    #[test]
    fn detect_loader_from_uid() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        assert_eq!(pack.detect_loader(), LoaderKind::Vanilla);

        pack.set_component("net.fabricmc.fabric-loader", "0.15.7", true);
        assert_eq!(pack.detect_loader(), LoaderKind::Fabric);
        assert_eq!(
            pack.detect_loader_component().map(|c| c.uid.as_str()),
            Some("net.fabricmc.fabric-loader")
        );
    }

    #[test]
    fn detect_loader_neoforge_not_misread_as_forge() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.4", true);
        pack.set_component("net.neoforged", "20.4.237", true);
        // uid 一等公民:不会像子串猜测那样把 neoforged 误判成 forge。
        assert_eq!(pack.detect_loader(), LoaderKind::NeoForge);
    }

    #[test]
    fn intermediary_is_not_a_loader() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        pack.components.push(Component::dependency(UID_FABRIC_INTERMEDIARY, Some("1.20.1".into())));
        // dependency_only 的 intermediary 不应被当作 loader。
        assert_eq!(pack.detect_loader(), LoaderKind::Vanilla);
    }

    #[test]
    fn disabled_loader_not_detected() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        pack.set_component("net.fabricmc.fabric-loader", "0.15.7", true);
        pack.get_component_mut("net.fabricmc.fabric-loader").unwrap().disabled = true;
        assert_eq!(pack.detect_loader(), LoaderKind::Vanilla);
    }

    // ---- loader_uid / known_loader 表 ----

    #[test]
    fn loader_uid_table_maps_both_directions() {
        assert_eq!(loader_uid(LoaderKind::Fabric), Some("net.fabricmc.fabric-loader"));
        assert_eq!(loader_uid(LoaderKind::NeoForge), Some("net.neoforged"));
        assert_eq!(loader_uid(LoaderKind::Vanilla), None);
        assert_eq!(loader_uid(LoaderKind::OptiFine), None);

        assert_eq!(known_loader("net.neoforged").unwrap().kind, LoaderKind::NeoForge);
        assert!(known_loader("net.minecraft").is_none(), "vanilla 不是 loader");
        assert!(known_loader("org.lwjgl3").is_none(), "lwjgl 不是 loader");
    }

    // ---- loader_conflict ----

    #[test]
    fn detects_double_loader_conflict() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        pack.set_component("net.fabricmc.fabric-loader", "0.15.7", true);
        assert!(pack.loader_conflict().is_none());
        pack.set_component("net.minecraftforge", "47.2.0", true);
        assert!(pack.loader_conflict().is_some());
    }

    // ---- from_loader ----

    #[test]
    fn from_loader_builds_two_component_pack() {
        let pack = PackProfile::from_loader("1.20.1", LoaderKind::Fabric, Some("0.15.7"));
        assert_eq!(pack.components.len(), 2);
        assert_eq!(pack.components[0].uid, UID_MINECRAFT);
        assert!(pack.components[0].important);
        assert_eq!(pack.components[1].uid, "net.fabricmc.fabric-loader");
        assert!(pack.components[1].important);
        assert_eq!(pack.detect_loader(), LoaderKind::Fabric);
    }

    #[test]
    fn from_loader_vanilla_is_single_component() {
        let pack = PackProfile::from_loader("1.21", LoaderKind::Vanilla, None);
        assert_eq!(pack.components.len(), 1);
        assert_eq!(pack.components[0].uid, UID_MINECRAFT);
        assert_eq!(pack.detect_loader(), LoaderKind::Vanilla);
    }

    // ---- resolver ----

    #[test]
    fn resolve_injects_fabric_intermediary_equal_to_mc() {
        let mut pack = PackProfile::from_loader("1.20.1", LoaderKind::Fabric, Some("0.15.7"));
        let changed = pack.resolve();
        assert!(changed, "首轮应注入 intermediary");

        // intermediary 被注入、版本 == mc、是 dependency_only。
        let inter = pack.get_component(UID_FABRIC_INTERMEDIARY).expect("应注入 intermediary");
        assert_eq!(inter.version.as_deref(), Some("1.20.1"));
        assert!(inter.dependency_only);

        // 顺序:intermediary 必须在 fabric-loader 之前(保证合并序)。
        let pos_inter = pack.components.iter().position(|c| c.uid == UID_FABRIC_INTERMEDIARY).unwrap();
        let pos_loader = pack.components.iter().position(|c| c.uid == "net.fabricmc.fabric-loader").unwrap();
        assert!(pos_inter < pos_loader, "intermediary 应夹在 vanilla 与 loader 之间");

        // 幂等:再次 resolve 不应再改动。
        assert!(!pack.resolve(), "已稳定,二次 resolve 不应改动");
    }

    #[test]
    fn resolve_injects_quilt_hashed() {
        let mut pack = PackProfile::from_loader("1.20.1", LoaderKind::Quilt, Some("0.20.0"));
        pack.resolve();
        let hashed = pack.get_component(UID_QUILT_HASHED).expect("应注入 hashed");
        assert_eq!(hashed.version.as_deref(), Some("1.20.1"));
    }

    #[test]
    fn resolve_corrects_stale_intermediary_version() {
        // intermediary 已存在但版本与 mc 不符(如改了 mc 版本)→ 解析器应改回 mc 版本。
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.4", true);
        pack.set_component("net.fabricmc.fabric-loader", "0.15.7", true);
        // 故意放一个过期版本的 intermediary。
        let mut stale = Component::dependency(UID_FABRIC_INTERMEDIARY, Some("1.20.1".into()));
        stale.cached_version = Some("1.20.1".into());
        pack.components.insert(0, stale);

        pack.resolve();
        assert_eq!(
            pack.get_component(UID_FABRIC_INTERMEDIARY).unwrap().version.as_deref(),
            Some("1.20.4"),
            "改 mc 版本后 intermediary 应级联到新版本"
        );
    }

    #[test]
    fn resolve_removes_orphaned_volatile_dependency() {
        // 一个 volatile dependency_only 组件,但无人依赖它 → 应被移除。
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        // 无 loader,intermediary 没有依赖者。
        pack.components.push(Component::dependency(UID_FABRIC_INTERMEDIARY, Some("1.20.1".into())));
        assert!(pack.has_component(UID_FABRIC_INTERMEDIARY));

        pack.resolve();
        assert!(
            !pack.has_component(UID_FABRIC_INTERMEDIARY),
            "无人依赖的 volatile 依赖项应被平凡移除"
        );
    }

    #[test]
    fn resolve_keeps_non_volatile_dependency() {
        // 非 volatile 的 dependency_only 不应被自动移除(用户可能显式保留)。
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        let mut dep = Component::new(UID_FABRIC_INTERMEDIARY, Some("1.20.1".into()));
        dep.dependency_only = true;
        dep.cached_volatile = false;
        pack.components.push(dep);

        pack.resolve();
        assert!(pack.has_component(UID_FABRIC_INTERMEDIARY), "非 volatile 依赖项应保留");
    }

    #[test]
    fn resolve_without_minecraft_anchor_is_noop() {
        let mut pack = PackProfile::new();
        pack.set_component("net.fabricmc.fabric-loader", "0.15.7", true);
        assert!(!pack.resolve(), "无 net.minecraft 锚版本时无可解析");
    }

    #[test]
    fn resolve_respects_cached_requires_from_version_file() {
        // Forge 的版本文件携带 net.minecraft==mc 的 cached_requires;mc 已存在则不注入,
        // 版本一致则不改动 → 稳定。
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        let mut forge = Component::important("net.minecraftforge", Some("47.2.0".into()));
        forge.cached_requires = vec![Require::equals(UID_MINECRAFT, "1.20.1")];
        pack.components.push(forge);

        assert!(!pack.resolve(), "mc 已满足 forge 的 equals 约束,应稳定无改动");
        assert_eq!(pack.minecraft_version(), Some("1.20.1"));
    }
