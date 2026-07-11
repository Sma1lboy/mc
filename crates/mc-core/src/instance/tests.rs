    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn realm_loader_kind_buckets_every_family_not_just_the_old_four() {
        // Regression: the hand-written match dropped liteloader/optifine into the
        // Vanilla bucket. Routing through LoaderKind::from_family fixes that.
        assert_eq!(loader_kind_from_str(Some("liteloader")), LoaderKind::LiteLoader);
        assert_eq!(loader_kind_from_str(Some("optifine")), LoaderKind::OptiFine);
        assert_eq!(loader_kind_from_str(Some("NeoForge")), LoaderKind::NeoForge);
        // Unknown / absent still defaults to Vanilla (a realm stub need not name one).
        assert_eq!(loader_kind_from_str(Some("rift")), LoaderKind::Vanilla);
        assert_eq!(loader_kind_from_str(None), LoaderKind::Vanilla);
    }

    /// 在临时目录里搭一个假的 game root,测试结束自动清理。
    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("mc-core-instance-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        /// 写入 versions/<id>/<id>.json(可选附带 inheritsFrom)。
        fn add_version(&self, id: &str, inherits: Option<&str>) {
            let paths = GamePaths::new(self.path.clone());
            let dir = paths.version_dir(id);
            fs::create_dir_all(&dir).unwrap();
            let json = match inherits {
                Some(p) => format!(r#"{{"id":"{id}","inheritsFrom":"{p}"}}"#),
                None => format!(r#"{{"id":"{id}"}}"#),
            };
            fs::write(paths.version_json(id), json).unwrap();
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn missing_versions_dir_is_empty() {
        let root = TempRoot::new("empty");
        let paths = GamePaths::new(root.path.clone());
        assert!(list_instances(&paths).is_empty());
    }


    #[test]
    fn hides_inherited_cores_lists_leaf_instances() {
        let root = TempRoot::new("mixed");
        // 共享原版核心:被 fabric 与 forge 两个实例 inheritsFrom → 是依赖,应从列表隐藏。
        root.add_version("1.20.1", None);
        root.add_version("fabric-loader-0.15.7-1.20.1", Some("1.20.1"));
        root.add_version("1.20.1-forge-47.2.0", Some("1.20.1"));
        // 没有任何目录继承它的独立原版实例(叶子)→ 应保留。
        root.add_version("1.18.2", None);

        // 一个没有版本 json 的目录(如 natives 残留),应被跳过。
        fs::create_dir_all(root.path.join("versions").join("junk")).unwrap();
        // 一个 json 损坏的目录,应被跳过而不 panic。
        let bad_dir = root.path.join("versions").join("broken");
        fs::create_dir_all(&bad_dir).unwrap();
        fs::write(bad_dir.join("broken.json"), "{ not valid json ").unwrap();

        let paths = GamePaths::new(root.path.clone());
        let list = list_instances(&paths);

        // 隐藏被继承的 1.20.1 核心;保留两个 loader 叶子 + 独立原版叶子 = 3。
        assert_eq!(list.len(), 3, "shared 1.20.1 core hidden; 3 leaf instances remain");

        let by_id = |id: &str| list.iter().find(|s| s.id == id).cloned();

        assert!(
            by_id("1.20.1").is_none(),
            "vanilla core inherited by other instances is hidden, not a phantom instance",
        );

        let standalone = by_id("1.18.2").expect("standalone vanilla leaf is listed");
        assert_eq!(standalone.loader, LoaderKind::Vanilla);
        assert_eq!(standalone.mc_version, "1.18.2");
        assert!(standalone.loader_version.is_none());

        let fabric = by_id("fabric-loader-0.15.7-1.20.1").expect("fabric leaf listed");
        assert_eq!(fabric.loader, LoaderKind::Fabric);
        assert_eq!(fabric.mc_version, "1.20.1");
        assert!(fabric.loader_version.is_some());

        let forge = by_id("1.20.1-forge-47.2.0").expect("forge leaf listed");
        assert_eq!(forge.loader, LoaderKind::Forge);
        assert_eq!(forge.mc_version, "1.20.1");
    }

    #[test]
    fn instance_name_comes_from_config() {
        let root = TempRoot::new("named");
        root.add_version("1.20.1", None);

        let inst = Instance::new("1.20.1", root.path.clone());
        let cfg = InstanceConfig {
            name: Some("Survival World".to_string()),
            ..Default::default()
        };
        inst.save_config(&cfg).unwrap();

        let paths = GamePaths::new(root.path.clone());
        let list = list_instances(&paths);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Survival World");
        assert_eq!(list[0].id, "1.20.1");
    }

    #[test]
    fn instance_config_roundtrip_via_helper() {
        let root = TempRoot::new("cfg");
        let inst = Instance::new("test-id", root.path.clone());

        // 未写盘时返回默认。
        let def = inst.load_config().unwrap();
        assert_eq!(def, InstanceConfig::default());
        assert_eq!(inst.version_id(), "test-id");
        assert!(inst.config_path().ends_with("versions/test-id/instance.json"));

        let cfg = InstanceConfig {
            memory_mb: 6144,
            ..Default::default()
        };
        inst.save_config(&cfg).unwrap();
        assert_eq!(inst.load_config().unwrap().memory_mb, 6144);
    }

    #[test]
    fn base64_encode_matches_known_vectors() {
        // RFC 4648 测试向量,覆盖三种填充情形。
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn detect_icon_reads_png_into_data_url() {
        let root = TempRoot::new("icon");
        root.add_version("1.20.1", None);
        let paths = GamePaths::new(root.path.clone());
        let dir = paths.version_dir("1.20.1");

        // 无 icon.png 时返回 None。
        assert!(detect_icon(&dir).is_none());

        // 写入图标后应被探测并内联为 data URL。
        fs::write(dir.join("icon.png"), b"abc").unwrap();
        assert_eq!(
            detect_icon(&dir).as_deref(),
            Some("data:image/png;base64,YWJj"),
        );

        // 该实例的列表项也应带上同一个 data URL。
        let list = list_instances(&paths);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].icon.as_deref(), Some("data:image/png;base64,YWJj"));
    }

    #[test]
    fn infer_loader_precedence() {
        assert_eq!(infer_loader("neoforge-1.21", None), LoaderKind::NeoForge);
        // neoforge 含 "forge" 子串,但应判定为 NeoForge。
        assert_eq!(infer_loader("1.21-neoforge-21.0.0", Some("1.21")), LoaderKind::NeoForge);
        assert_eq!(infer_loader("1.20.1-forge-47.2.0", Some("1.20.1")), LoaderKind::Forge);
        assert_eq!(infer_loader("fabric-loader-0.15", Some("1.20.1")), LoaderKind::Fabric);
        assert_eq!(infer_loader("quilt-loader-0.20", Some("1.20.1")), LoaderKind::Quilt);
        assert_eq!(infer_loader("1.20.1-OptiFine_HD_U_I6", None), LoaderKind::OptiFine);
        assert_eq!(infer_loader("1.5.2-LiteLoader1.5.2", None), LoaderKind::LiteLoader);
        assert_eq!(infer_loader("1.20.1", None), LoaderKind::Vanilla);
    }
