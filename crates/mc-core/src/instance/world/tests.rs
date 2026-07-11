    use super::*;
    use std::io::Write;

    /// 临时 game root,Drop 时自动清理。
    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "mc-core-world-test-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn instance(&self) -> Instance {
            // version_id 不影响 saves 路径解析,这里给个占位 id。
            Instance::new("test-version", self.path.clone())
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// 构造一棵最小 level.dat 的 NBT 树:root → Data{LevelName, GameType, LastPlayed, RandomSeed}。
    fn build_level_value(name: &str, game_type: i32, last_played: i64, seed: i64) -> Value {
        let mut data = HashMap::new();
        data.insert("LevelName".to_string(), Value::String(name.to_string()));
        data.insert("GameType".to_string(), Value::Int(game_type));
        data.insert("LastPlayed".to_string(), Value::Long(last_played));
        data.insert("RandomSeed".to_string(), Value::Long(seed));

        let mut root = HashMap::new();
        root.insert("Data".to_string(), Value::Compound(data));
        Value::Compound(root)
    }

    /// 把一棵 NBT 树序列化 + gzip 写到 saves/<world>/level.dat。
    fn write_world(inst: &Instance, world: &str, value: &Value) {
        let dir = inst.saves_dir().join(world);
        std::fs::create_dir_all(&dir).unwrap();
        let nbt = fastnbt::to_bytes(value).unwrap();
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&nbt).unwrap();
        let gz = enc.finish().unwrap();
        std::fs::write(dir.join(LEVEL_DAT), gz).unwrap();
    }

    #[test]
    fn lists_world_with_parsed_name_and_mode() {
        let root = TempRoot::new("list");
        let inst = root.instance();

        let value = build_level_value("My Creative World", 1, 1_700_000_000_000, 42);
        write_world(&inst, "world1", &value);

        let worlds = list_worlds(&inst);
        assert_eq!(worlds.len(), 1);
        let w = &worlds[0];
        assert_eq!(w.folder, "world1");
        assert_eq!(w.name, "My Creative World");
        assert_eq!(w.game_mode, "creative");
        assert_eq!(w.last_played, 1_700_000_000_000);
        assert_eq!(w.seed, Some(42));
        assert!(w.size_bytes > 0, "size should count level.dat bytes");
    }

    #[test]
    fn import_world_zip_roundtrips_from_backup() {
        let root = TempRoot::new("import");
        let inst = root.instance();
        write_world(&inst, "world1", &build_level_value("Round Trip", 0, 123, 9));

        // 备份得到 world1-backup.zip(zip 内根目录为 world1/)。
        let zip = backup_world(&inst, "world1", &root.path.join("backups/world1-backup.zip")).unwrap();
        assert!(zip.is_file());

        // 导入:文件名去 -backup → world1,已存在 → 唯一化为 world1-2。
        let folder = import_world_zip(&inst, &zip).unwrap();
        assert_eq!(folder, "world1-2");
        assert!(inst.saves_dir().join("world1-2").join(LEVEL_DAT).is_file());

        // 现在有两个世界,导入的那个元数据可正常解析。
        let worlds = list_worlds(&inst);
        assert_eq!(worlds.len(), 2);
        let imported = worlds.iter().find(|w| w.folder == "world1-2").unwrap();
        assert_eq!(imported.name, "Round Trip");
    }

    #[test]
    fn import_world_zip_rejects_non_world() {
        let root = TempRoot::new("import-bad");
        let inst = root.instance();
        // 造一个不含 level.dat 的 zip。
        let zip_path = root.path.join("notworld.zip");
        let f = std::fs::File::create(&zip_path).unwrap();
        let mut zw = ZipWriter::new(f);
        zw.start_file("readme.txt", SimpleFileOptions::default()).unwrap();
        zw.write_all(b"hi").unwrap();
        zw.finish().unwrap();

        assert!(import_world_zip(&inst, &zip_path).is_err());
        // 失败不应留下任何世界目录。
        assert!(list_worlds(&inst).is_empty());
    }

    #[test]
    fn survival_mode_mapping() {
        let root = TempRoot::new("survival");
        let inst = root.instance();
        write_world(&inst, "s", &build_level_value("S", 0, 0, 7));
        let worlds = list_worlds(&inst);
        assert_eq!(worlds[0].game_mode, "survival");
    }

    #[test]
    fn corrupt_level_dat_still_listed_as_unknown() {
        let root = TempRoot::new("corrupt");
        let inst = root.instance();
        // 写一个非 gzip / 非 NBT 的 level.dat:解析必然失败,但世界应仍被列出。
        let dir = inst.saves_dir().join("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LEVEL_DAT), b"not a valid level.dat").unwrap();

        let worlds = list_worlds(&inst);
        assert_eq!(worlds.len(), 1);
        assert_eq!(worlds[0].folder, "broken");
        assert_eq!(worlds[0].name, "broken", "name falls back to folder");
        assert_eq!(worlds[0].game_mode, "unknown");
        assert_eq!(worlds[0].seed, None);
    }

    #[test]
    fn directory_without_level_dat_is_skipped() {
        let root = TempRoot::new("skip");
        let inst = root.instance();
        // 空目录(无 level.dat)不算世界。
        std::fs::create_dir_all(inst.saves_dir().join("not-a-world")).unwrap();
        write_world(&inst, "real", &build_level_value("Real", 0, 0, 1));

        let worlds = list_worlds(&inst);
        assert_eq!(worlds.len(), 1);
        assert_eq!(worlds[0].folder, "real");
    }

    #[test]
    fn missing_saves_dir_returns_empty() {
        let root = TempRoot::new("nosaves");
        let inst = root.instance();
        assert!(list_worlds(&inst).is_empty());
    }

    #[test]
    fn worldgensettings_seed_preferred_over_random_seed() {
        let root = TempRoot::new("seed");
        let inst = root.instance();

        // 同时存在 WorldGenSettings.seed 与 RandomSeed,应取前者(1.16+ 格式)。
        let mut data = HashMap::new();
        data.insert("LevelName".to_string(), Value::String("Seeded".to_string()));
        data.insert("GameType".to_string(), Value::Int(0));
        data.insert("RandomSeed".to_string(), Value::Long(111));
        let mut wgs = HashMap::new();
        wgs.insert("seed".to_string(), Value::Long(999));
        data.insert("WorldGenSettings".to_string(), Value::Compound(wgs));
        let mut rootmap = HashMap::new();
        rootmap.insert("Data".to_string(), Value::Compound(data));

        write_world(&inst, "w", &Value::Compound(rootmap));

        let worlds = list_worlds(&inst);
        assert_eq!(worlds[0].seed, Some(999));
    }

    #[test]
    fn rename_changes_name_preserves_other_tags() {
        let root = TempRoot::new("rename");
        let inst = root.instance();

        // 原始世界带种子/模式/时间,重命名后这些都应保留。
        write_world(&inst, "w", &build_level_value("Old Name", 2, 12345, 678));

        rename_world(&inst, "w", "New Name").unwrap();

        let worlds = list_worlds(&inst);
        assert_eq!(worlds.len(), 1);
        let w = &worlds[0];
        assert_eq!(w.name, "New Name");
        // 其它标签必须无损保留。
        assert_eq!(w.game_mode, "adventure");
        assert_eq!(w.last_played, 12345);
        assert_eq!(w.seed, Some(678));
    }

    #[test]
    fn backup_creates_zip_with_world_contents() {
        let root = TempRoot::new("backup");
        let inst = root.instance();

        write_world(&inst, "w", &build_level_value("W", 0, 0, 1));
        // 加一个嵌套文件,验证递归打包。
        let region = inst.saves_dir().join("w").join("region");
        std::fs::create_dir_all(&region).unwrap();
        std::fs::write(region.join("r.0.0.mca"), b"chunk-bytes").unwrap();

        let dest = root.path.join("backups").join("w-backup.zip");
        let zip_path = backup_world(&inst, "w", &dest).unwrap();

        assert!(zip_path.exists());
        assert_eq!(zip_path.file_name().unwrap(), "w-backup.zip");

        // 读回 zip,确认包含世界根目录下的 level.dat 与嵌套 region 文件。
        let f = std::fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();
        assert!(names.iter().any(|n| n == "w/level.dat"), "names: {names:?}");
        assert!(
            names.iter().any(|n| n == "w/region/r.0.0.mca"),
            "names: {names:?}"
        );
    }

    #[test]
    fn delete_removes_world_directory() {
        let root = TempRoot::new("delete");
        let inst = root.instance();
        write_world(&inst, "w", &build_level_value("W", 0, 0, 1));
        let dir = inst.saves_dir().join("w");
        assert!(dir.exists());

        delete_world(&inst, "w").unwrap();
        assert!(!dir.exists(), "world dir should be gone after delete");
    }

    #[test]
    fn rejects_path_traversal_folder() {
        let root = TempRoot::new("traversal");
        let inst = root.instance();
        // folder 含 .. / 分隔符必须被拒,delete/backup/rename 都不能逃出 saves/。
        assert!(delete_world(&inst, "../../etc").is_err());
        assert!(rename_world(&inst, "sub/world", "x").is_err());
        assert!(backup_world(&inst, "..", &root.path.join("b.zip")).is_err());
    }

    #[test]
    fn operations_on_missing_world_error() {
        let root = TempRoot::new("missing");
        let inst = root.instance();
        assert!(matches!(
            backup_world(&inst, "nope", &root.path),
            Err(CoreError::InstanceNotFound(_))
        ));
        assert!(matches!(
            delete_world(&inst, "nope"),
            Err(CoreError::InstanceNotFound(_))
        ));
        assert!(matches!(
            rename_world(&inst, "nope", "x"),
            Err(CoreError::InstanceNotFound(_))
        ));
    }
