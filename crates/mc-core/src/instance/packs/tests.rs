    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// 在临时目录里搭一个最小 game root,测试结束自动清理。
    /// 结构:`<root>/versions/<id>/resourcepacks/...`(game_dir == version_dir)。
    struct TempInst {
        root: PathBuf,
        inst: Instance,
    }

    impl TempInst {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("mc-core-packs-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let inst = Instance::new("1.20.1", root.clone());
            // 预建资源包目录,模拟已有实例。
            fs::create_dir_all(inst.resourcepacks_dir()).unwrap();
            Self { root, inst }
        }
    }

    impl Drop for TempInst {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn lists_with_enabled_flag_and_count() {
        let t = TempInst::new("list");
        let rp = t.inst.resourcepacks_dir();
        // 一个启用的、一个禁用的。
        fs::write(rp.join("Faithful.zip"), b"PK\x03\x04not-a-real-zip").unwrap();
        fs::write(rp.join("OldPack.zip.disabled"), b"PK\x03\x04not-a-real-zip").unwrap();
        // 一个无关文件(txt),应被忽略。
        fs::write(rp.join("readme.txt"), b"ignore me").unwrap();

        let packs = list_packs(&t.inst, PackKind::ResourcePack, None);
        assert_eq!(packs.len(), 2, "只应列出两个 zip 包,忽略 txt");

        let faithful = packs.iter().find(|p| p.file_name == "Faithful.zip").unwrap();
        assert!(faithful.enabled, "无 .disabled 后缀应为启用");
        assert_eq!(faithful.kind, "resourcepack");
        assert!(faithful.size > 0);

        let old = packs.iter().find(|p| p.file_name == "OldPack.zip.disabled").unwrap();
        assert!(!old.enabled, ".disabled 后缀应为禁用");
    }

    #[test]
    fn set_enabled_renames_off_and_on() {
        let t = TempInst::new("toggle");
        let rp = t.inst.resourcepacks_dir();
        fs::write(rp.join("Pack.zip"), b"data").unwrap();

        // 禁用:应改名为 Pack.zip.disabled。
        set_pack_enabled(&t.inst, PackKind::ResourcePack, "Pack.zip", false, None).unwrap();
        assert!(!rp.join("Pack.zip").exists());
        assert!(rp.join("Pack.zip.disabled").exists());

        let after_disable = list_packs(&t.inst, PackKind::ResourcePack, None);
        assert_eq!(after_disable.len(), 1);
        assert!(!after_disable[0].enabled);

        // 再启用:应改回 Pack.zip。
        set_pack_enabled(&t.inst, PackKind::ResourcePack, "Pack.zip.disabled", true, None).unwrap();
        assert!(rp.join("Pack.zip").exists());
        assert!(!rp.join("Pack.zip.disabled").exists());

        let after_enable = list_packs(&t.inst, PackKind::ResourcePack, None);
        assert!(after_enable[0].enabled);
    }

    #[test]
    fn set_enabled_is_idempotent() {
        let t = TempInst::new("idem");
        let rp = t.inst.resourcepacks_dir();
        fs::write(rp.join("A.zip"), b"x").unwrap();

        // 已启用再启用:空操作,文件不变。
        set_pack_enabled(&t.inst, PackKind::ResourcePack, "A.zip", true, None).unwrap();
        assert!(rp.join("A.zip").exists());
    }

    #[test]
    fn rejects_path_traversal_filename() {
        let t = TempInst::new("traversal");
        let err = set_pack_enabled(&t.inst, PackKind::ResourcePack, "../evil.zip", true, None);
        assert!(err.is_err(), "含 .. 的文件名必须被拒绝");
        let err2 = set_pack_enabled(&t.inst, PackKind::ResourcePack, "sub/evil.zip", false, None);
        assert!(err2.is_err(), "含分隔符的文件名必须被拒绝");
    }

    #[test]
    fn datapack_lists_directories() {
        let t = TempInst::new("datapack-dir");
        let dp = t.inst.datapacks_dir();
        fs::create_dir_all(dp.join("MyDatapack")).unwrap();
        fs::write(dp.join("vanilla-tweaks.zip"), b"PK").unwrap();

        let packs = list_packs(&t.inst, PackKind::Datapack, None);
        assert_eq!(packs.len(), 2, "数据包应同时列出目录形态与 zip 形态");
        assert!(packs.iter().any(|p| p.file_name == "MyDatapack"));
        assert!(packs.iter().any(|p| p.file_name == "vanilla-tweaks.zip"));
    }

    #[test]
    fn missing_dir_lists_empty() {
        let t = TempInst::new("missing");
        // shaderpacks 目录未创建。
        let packs = list_packs(&t.inst, PackKind::Shader, None);
        assert!(packs.is_empty());
    }

    #[test]
    fn reads_resourcepack_description() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let t = TempInst::new("mcmeta");
        let rp = t.inst.resourcepacks_dir();
        let zip_path = rp.join("Described.zip");

        // 写一个真实的 zip,内含 pack.mcmeta。
        let file = fs::File::create(&zip_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        zw.start_file("pack.mcmeta", SimpleFileOptions::default()).unwrap();
        zw.write_all(br#"{"pack":{"pack_format":15,"description":"A cool pack"}}"#)
            .unwrap();
        zw.finish().unwrap();

        let packs = list_packs(&t.inst, PackKind::ResourcePack, None);
        let described = packs.iter().find(|p| p.file_name == "Described.zip").unwrap();
        assert_eq!(described.description.as_deref(), Some("A cool pack"));
    }

    #[test]
    fn pack_kind_maps_to_dir_and_resource_kind() {
        let t = TempInst::new("kindmap");
        assert_eq!(PackKind::ResourcePack.dir(&t.inst), t.inst.resourcepacks_dir());
        assert_eq!(PackKind::Shader.dir(&t.inst), t.inst.shaderpacks_dir());
        assert_eq!(PackKind::Datapack.dir(&t.inst), t.inst.datapacks_dir());

        assert_eq!(PackKind::ResourcePack.to_resource_kind(), ResourceKind::ResourcePack);
        assert_eq!(PackKind::Shader.to_resource_kind(), ResourceKind::Shader);
        assert_eq!(PackKind::Datapack.to_resource_kind(), ResourceKind::Datapack);
    }

    #[test]
    fn delete_removes_file() {
        let t = TempInst::new("delete");
        let rp = t.inst.resourcepacks_dir();
        let target = rp.join("Trash.zip");
        fs::write(&target, b"bye").unwrap();

        delete_pack(&t.inst, PackKind::ResourcePack, "Trash.zip", None).unwrap();
        assert!(!target.exists(), "删除后文件应不在原位(回收站或硬删均可)");

        // 重复删除不存在的文件应幂等成功。
        delete_pack(&t.inst, PackKind::ResourcePack, "Trash.zip", None).unwrap();
    }
