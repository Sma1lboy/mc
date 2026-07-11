    use super::*;
    use std::io::Write;

    fn write_zip(dest: &Path, files: &[(&str, &[u8])]) {
        let out = std::fs::File::create(dest).unwrap();
        let mut zw = zip::ZipWriter::new(out);
        let opt = zip::write::SimpleFileOptions::default();
        for (name, body) in files {
            zw.start_file(*name, opt).unwrap();
            zw.write_all(body).unwrap();
        }
        zw.finish().unwrap();
    }

    struct Tmp {
        dir: PathBuf,
    }
    impl Tmp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("mc-core-archive-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn index_lists_only_files_and_normalizes() {
        let t = Tmp::new("index");
        let zp = t.dir.join("p.zip");
        write_zip(&zp, &[("a/b.txt", b"x"), ("c.txt", b"y")]);
        let idx = ZipArchiveIndex::open(&zp).unwrap();
        use super::super::ArchiveIndex;
        let entries = idx.entries();
        assert!(entries.contains(&"a/b.txt".to_string()));
        assert!(entries.contains(&"c.txt".to_string()));
    }

    #[test]
    fn extract_subtree_with_root_strips_prefix() {
        let t = Tmp::new("subtree");
        let zp = t.dir.join("p.zip");
        write_zip(&zp, &[("Pack/manifest.json", b"M"), ("Pack/mods/a.jar", b"J"), ("other.txt", b"O")]);
        let mut idx = ZipArchiveIndex::open(&zp).unwrap();
        let staging = t.dir.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        idx.extract_subtree("Pack", &staging).unwrap();
        assert_eq!(std::fs::read(staging.join("manifest.json")).unwrap(), b"M");
        assert_eq!(std::fs::read(staging.join("mods/a.jar")).unwrap(), b"J");
        // Pack 之外的 other.txt 不在子树内,不应解出。
        assert!(!staging.join("other.txt").exists());
    }

    #[test]
    fn extract_subtree_empty_root_extracts_all() {
        let t = Tmp::new("rootall");
        let zp = t.dir.join("p.zip");
        write_zip(&zp, &[("modrinth.index.json", b"I"), ("overrides/x.txt", b"X")]);
        let mut idx = ZipArchiveIndex::open(&zp).unwrap();
        let staging = t.dir.join("s");
        std::fs::create_dir_all(&staging).unwrap();
        idx.extract_subtree("", &staging).unwrap();
        assert_eq!(std::fs::read(staging.join("modrinth.index.json")).unwrap(), b"I");
        assert_eq!(std::fs::read(staging.join("overrides/x.txt")).unwrap(), b"X");
    }

    #[test]
    fn extract_prefix_blocks_zip_slip() {
        let t = Tmp::new("slip");
        let zp = t.dir.join("evil.zip");
        write_zip(&zp, &[("overrides/../../escaped.txt", b"PWNED")]);
        let mut idx = ZipArchiveIndex::open(&zp).unwrap();
        let dest = t.dir.join("game");
        std::fs::create_dir_all(&dest).unwrap();
        let err = idx.extract_prefix("overrides/", &dest).unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "zip-slip 应被拒绝");
    }

    #[test]
    fn prepared_index_caches_manifest_for_read_small() {
        let t = Tmp::new("prep");
        let zp = t.dir.join("p.zip");
        write_zip(&zp, &[("manifest.json", br#"{"addons":[]}"#), ("overrides/a.txt", b"x")]);
        let prepared = PackArchive::open(&zp).unwrap().into_prepared(&["manifest.json"]);
        use super::super::ArchiveIndex;
        assert_eq!(prepared.read_small("manifest.json").unwrap(), br#"{"addons":[]}"#.to_vec());
        // 未预取的条目读不到(返回 None,而非读盘)。
        assert!(prepared.read_small("overrides/a.txt").is_none());
        // 取回 inner 仍可解压子树。
        let mut inner = prepared.into_inner();
        let s = t.dir.join("s");
        std::fs::create_dir_all(&s).unwrap();
        inner.extract_subtree("overrides", &s).unwrap();
        assert_eq!(std::fs::read(s.join("a.txt")).unwrap(), b"x");
    }

    #[test]
    fn dir_archive_indexes_files_and_extracts_subtree() {
        // 未解压的 Prism 实例目录:DirArchiveIndex 应能列条目、读 manifest、按子树拷贝。
        let t = Tmp::new("dir");
        let inst = t.dir.join("MyInstance");
        std::fs::create_dir_all(inst.join(".minecraft/mods")).unwrap();
        std::fs::write(inst.join("mmc-pack.json"), br#"{"formatVersion":1,"components":[]}"#).unwrap();
        std::fs::write(inst.join("instance.cfg"), b"name=X\n").unwrap();
        std::fs::write(inst.join(".minecraft/mods/a.jar"), b"J").unwrap();

        let idx = DirArchiveIndex::open(&inst).unwrap();
        let entries = &idx.entries;
        assert!(entries.contains(&"mmc-pack.json".to_string()));
        assert!(entries.contains(&".minecraft/mods/a.jar".to_string()));
        assert_eq!(idx.read_entry("instance.cfg").unwrap(), b"name=X\n".to_vec());

        // 经统一 PackArchive::open(目录) → prepared → extract_subtree。
        let mut inner = PackArchive::open(&inst).unwrap().into_prepared(&["mmc-pack.json"]).into_inner();
        let staging = t.dir.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        inner.extract_subtree("", &staging).unwrap();
        assert_eq!(std::fs::read(staging.join("mmc-pack.json")).unwrap(), br#"{"formatVersion":1,"components":[]}"#.to_vec());
        assert_eq!(std::fs::read(staging.join(".minecraft/mods/a.jar")).unwrap(), b"J");
    }

    #[test]
    fn dir_archive_extract_subtree_with_root_strips_prefix() {
        // 目录里包根嵌套一层(Prism 导出常见):extract_subtree 按包根剥前缀。
        let t = Tmp::new("dirroot");
        let base = t.dir.join("export");
        std::fs::create_dir_all(base.join("MyInstance/.minecraft")).unwrap();
        std::fs::write(base.join("MyInstance/mmc-pack.json"), b"M").unwrap();
        std::fs::write(base.join("MyInstance/.minecraft/options.txt"), b"O").unwrap();
        std::fs::write(base.join("sibling.txt"), b"S").unwrap();

        let idx = DirArchiveIndex::open(&base).unwrap();
        let staging = t.dir.join("s");
        std::fs::create_dir_all(&staging).unwrap();
        idx.extract_subtree("MyInstance", &staging).unwrap();
        assert_eq!(std::fs::read(staging.join("mmc-pack.json")).unwrap(), b"M");
        assert_eq!(std::fs::read(staging.join(".minecraft/options.txt")).unwrap(), b"O");
        // 包根之外的文件不在子树内。
        assert!(!staging.join("sibling.txt").exists());
    }

    #[test]
    fn overlay_dir_safe_copies_tree() {
        let t = Tmp::new("overlay");
        let src = t.dir.join("overrides");
        std::fs::create_dir_all(src.join("config")).unwrap();
        std::fs::write(src.join("config/a.cfg"), b"k=1").unwrap();
        let game = t.dir.join("game");
        std::fs::create_dir_all(&game).unwrap();
        overlay_dir_safe(&src, &game).unwrap();
        assert_eq!(std::fs::read(game.join("config/a.cfg")).unwrap(), b"k=1");
    }
