    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use zip::write::SimpleFileOptions;

    /// 在临时目录搭一个最小实例(只需要 mods/ 目录),测试结束自动清理。
    struct TempInst {
        root: PathBuf,
        inst: Instance,
    }

    impl TempInst {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("mc-core-mods-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let inst = Instance::new("1.20.1", root.clone());
            fs::create_dir_all(inst.mods_dir()).unwrap();
            Self { root, inst }
        }
    }

    impl Drop for TempInst {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// 构造一个内含单个文本条目的最小 jar(zip)写到指定路径。
    fn write_jar_with_entry(path: &Path, entry_name: &str, content: &str) {
        let file = fs::File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        zw.start_file(entry_name, opts).unwrap();
        zw.write_all(content.as_bytes()).unwrap();
        zw.finish().unwrap();
    }

    #[test]
    fn lists_and_parses_fabric_mod() {
        let t = TempInst::new("fabric");
        let jar = t.inst.mods_dir().join("sodium.jar");
        let fmj = r#"{
            "schemaVersion": 1,
            "id": "sodium",
            "version": "0.5.3",
            "name": "Sodium",
            "description": "A rendering engine.",
            "authors": ["JellySquid", {"name": "Contributor X"}]
        }"#;
        write_jar_with_entry(&jar, "fabric.mod.json", fmj);

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        let m = &mods[0];
        assert_eq!(m.file_name, "sodium.jar");
        assert!(m.enabled);
        assert_eq!(m.name, "Sodium");
        assert_eq!(m.version.as_deref(), Some("0.5.3"));
        assert_eq!(m.mod_id.as_deref(), Some("sodium"));
        assert_eq!(m.loader, "fabric");
        assert_eq!(m.description.as_deref(), Some("A rendering engine."));
        assert_eq!(m.authors, vec!["JellySquid", "Contributor X"]);
        assert!(m.size > 0);
    }

    #[test]
    fn parses_quilt_mod() {
        let t = TempInst::new("quilt");
        let jar = t.inst.mods_dir().join("qmod.jar");
        let qmj = r#"{
            "schema_version": 1,
            "quilt_loader": {
                "id": "example_mod",
                "version": "1.2.3",
                "metadata": {
                    "name": "Example Quilt Mod",
                    "description": "Quilt test.",
                    "contributors": { "Alice": "Owner", "Bob": "Author" }
                }
            }
        }"#;
        write_jar_with_entry(&jar, "quilt.mod.json", qmj);

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        let m = &mods[0];
        assert_eq!(m.name, "Example Quilt Mod");
        assert_eq!(m.version.as_deref(), Some("1.2.3"));
        assert_eq!(m.mod_id.as_deref(), Some("example_mod"));
        assert_eq!(m.loader, "quilt");
        assert_eq!(m.description.as_deref(), Some("Quilt test."));
        let mut authors = m.authors.clone();
        authors.sort();
        assert_eq!(authors, vec!["Alice", "Bob"]);
    }

    #[test]
    fn parses_forge_mods_toml() {
        let t = TempInst::new("forge");
        let jar = t.inst.mods_dir().join("forgemod.jar");
        let toml = r#"
modLoader = "javafml"
loaderVersion = "[47,)"
license = "MIT"
authors = "Top Author"

[[mods]]
modId = "examplemod"
version = "1.0.0"
displayName = "Example Forge Mod"
authors = "Alice, Bob and Carol"
description = '''
A multi-line
forge description.
'''
"#;
        write_jar_with_entry(&jar, "META-INF/mods.toml", toml);

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        let m = &mods[0];
        assert_eq!(m.name, "Example Forge Mod");
        assert_eq!(m.version.as_deref(), Some("1.0.0"));
        assert_eq!(m.mod_id.as_deref(), Some("examplemod"));
        assert_eq!(m.loader, "forge");
        assert_eq!(m.authors, vec!["Alice", "Bob", "Carol"]);
        assert!(m.description.as_deref().unwrap().contains("multi-line"));
    }

    #[test]
    fn parses_neoforge_toml_and_prefers_it() {
        let t = TempInst::new("neoforge");
        let jar = t.inst.mods_dir().join("nfmod.jar");
        let toml = r#"
modLoader = "javafml"
[[mods]]
modId = "neomod"
version = "2.0.0"
displayName = "Neo Mod"
"#;
        write_jar_with_entry(&jar, "META-INF/neoforge.mods.toml", toml);

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].loader, "neoforge");
        assert_eq!(mods[0].mod_id.as_deref(), Some("neomod"));
        assert_eq!(mods[0].name, "Neo Mod");
    }

    #[test]
    fn unknown_jar_falls_back_to_filename() {
        let t = TempInst::new("unknown");
        let jar = t.inst.mods_dir().join("MysteryMod-1.0.jar");
        // 内含一个无关条目,既不是 fabric 也不是 forge。
        write_jar_with_entry(&jar, "pack.mcmeta", "{}");

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "MysteryMod-1.0");
        assert_eq!(mods[0].loader, "unknown");
        assert!(mods[0].enabled);
    }

    #[test]
    fn corrupt_jar_is_skipped_not_panicked() {
        let t = TempInst::new("corrupt");
        // 写一个非 zip 文件但以 .jar 结尾。
        fs::write(t.inst.mods_dir().join("broken.jar"), b"not a zip at all").unwrap();
        let mods = list_mods(&t.inst);
        // 不 panic,且退化为文件名条目。
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "broken");
        assert_eq!(mods[0].loader, "unknown");
    }

    #[test]
    fn disabled_state_detected_and_toggled() {
        let t = TempInst::new("toggle");
        let jar = t.inst.mods_dir().join("togglemod.jar");
        write_jar_with_entry(
            &jar,
            "fabric.mod.json",
            r#"{"id":"togglemod","name":"Toggle","version":"1.0"}"#,
        );

        // 初始启用。
        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        assert!(mods[0].enabled);
        assert_eq!(mods[0].file_name, "togglemod.jar");

        // 停用 → 文件应被重命名为 .disabled。
        set_mod_enabled(&t.inst, "togglemod.jar", false).unwrap();
        assert!(!t.inst.mods_dir().join("togglemod.jar").exists());
        assert!(t.inst.mods_dir().join("togglemod.jar.disabled").exists());

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        assert!(!mods[0].enabled);
        assert_eq!(mods[0].file_name, "togglemod.jar.disabled");
        // 即使停用,元数据仍应能从 jar 内读出。
        assert_eq!(mods[0].name, "Toggle");

        // 用带 .disabled 的文件名重新启用,且接受已是目标态时的幂等。
        set_mod_enabled(&t.inst, "togglemod.jar.disabled", true).unwrap();
        assert!(t.inst.mods_dir().join("togglemod.jar").exists());
        set_mod_enabled(&t.inst, "togglemod.jar", true).unwrap(); // 幂等 no-op
        assert!(t.inst.mods_dir().join("togglemod.jar").exists());
    }

    #[test]
    fn delete_removes_file() {
        let t = TempInst::new("delete");
        let jar = t.inst.mods_dir().join("doomed.jar");
        write_jar_with_entry(&jar, "fabric.mod.json", r#"{"id":"doomed"}"#);
        assert!(jar.exists());

        delete_mod(&t.inst, "doomed.jar").unwrap();
        assert!(!jar.exists());

        // 再删一次:文件已不存在,应幂等成功。
        delete_mod(&t.inst, "doomed.jar").unwrap();
    }

    #[test]
    fn rejects_path_traversal_file_name() {
        let t = TempInst::new("traversal");
        // 含 .. 或分隔符的文件名必须被拒绝,绝不逃出 mods/ 误删/改名其它文件。
        assert!(delete_mod(&t.inst, "../evil.jar").is_err());
        assert!(delete_mod(&t.inst, "sub/evil.jar").is_err());
        assert!(set_mod_enabled(&t.inst, "../evil.jar", false).is_err());
        assert!(set_mod_enabled(&t.inst, "a\\b.jar", true).is_err());
    }

    #[test]
    fn superseded_removes_same_mod_id_only() {
        let t = TempInst::new("supersede");
        let dir = t.inst.mods_dir();
        // 同一个 mod 的两个版本(相同 mod_id "sodium",不同文件名)。
        write_jar_with_entry(&dir.join("sodium-0.5.3.jar"), "fabric.mod.json", r#"{"id":"sodium","version":"0.5.3"}"#);
        write_jar_with_entry(&dir.join("sodium-0.5.8.jar"), "fabric.mod.json", r#"{"id":"sodium","version":"0.5.8"}"#);
        // 无关的另一个 mod,绝不能被动。
        write_jar_with_entry(&dir.join("fabric-api.jar"), "fabric.mod.json", r#"{"id":"fabric"}"#);
        // 同 mod 的旧版处于停用态,也应被清理(否则重新启用又冲突)。
        write_jar_with_entry(&dir.join("sodium-old.jar.disabled"), "fabric.mod.json", r#"{"id":"sodium","version":"0.4.0"}"#);

        let mut removed = remove_superseded(&t.inst, "sodium-0.5.8.jar").unwrap();
        removed.sort();

        assert!(dir.join("sodium-0.5.8.jar").exists(), "新版本应保留");
        assert!(dir.join("fabric-api.jar").exists(), "无关 mod 应保留");
        assert!(!dir.join("sodium-0.5.3.jar").exists(), "同 mod 旧版应清理");
        assert!(!dir.join("sodium-old.jar.disabled").exists(), "停用的同 mod 旧版也应清理");
        assert_eq!(
            removed,
            vec!["sodium-0.5.3.jar".to_string(), "sodium-old.jar.disabled".to_string()]
        );
    }

    #[test]
    fn superseded_noop_when_new_jar_unreadable() {
        // 新文件读不出 mod_id(损坏/无元数据)时,不得删除任何东西 —— 无法可靠判定归属。
        let t = TempInst::new("supersede-unknown");
        let dir = t.inst.mods_dir();
        write_jar_with_entry(&dir.join("sodium-0.5.3.jar"), "fabric.mod.json", r#"{"id":"sodium"}"#);
        fs::write(dir.join("mystery.jar"), b"not a zip").unwrap();

        let removed = remove_superseded(&t.inst, "mystery.jar").unwrap();
        assert!(removed.is_empty());
        assert!(dir.join("sodium-0.5.3.jar").exists(), "判定不了时旧文件必须原样保留");
    }

    #[test]
    fn missing_mods_dir_returns_empty() {
        let root = std::env::temp_dir()
            .join(format!("mc-core-mods-test-nomods-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let inst = Instance::new("1.20.1", root.clone());
        // 注意:不创建 mods 目录。
        assert!(list_mods(&inst).is_empty());
        let _ = fs::remove_dir_all(&root);
    }
