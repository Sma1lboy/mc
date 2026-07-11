    use super::*;

    #[test]
    fn sanitizes_illegal_and_reserved() {
        assert_eq!(sanitize_filename("my/cool:pack", '-'), "my-cool-pack");
        assert_eq!(sanitize_filename("trailing... ", '-'), "trailing");
        assert_eq!(sanitize_filename("CON", '-'), "CON_");
        assert_eq!(sanitize_filename("nul.txt", '-'), "nul.txt_");
        assert_eq!(sanitize_filename("", '-'), "-");
    }

    #[test]
    fn slugify_collapses_whitespace_keeps_unicode_and_falls_back() {
        // Whitespace / illegal chars collapse to single dashes; ends trimmed.
        assert_eq!(slugify("  My Cool / Pack  ", "x"), "My-Cool-Pack");
        // Unicode is preserved (a Chinese name is a valid dir name).
        assert_eq!(slugify("我的世界", "x"), "我的世界");
        // Empty/garbage falls back to the caller-chosen default.
        assert_eq!(slugify("   ///   ", "instance"), "instance");
        assert_eq!(slugify("", "World"), "World");
    }

    #[test]
    fn unique_name_suffixes_only_on_collision() {
        let taken = ["pack", "pack-2"];
        let exists = |c: &str| taken.contains(&c);
        // Free name is used verbatim.
        assert_eq!(unique_name("fresh", exists), "fresh");
        // First free suffix wins, skipping taken ones.
        assert_eq!(unique_name("pack", exists), "pack-3");
    }

    #[test]
    fn flags_bang_path_as_error() {
        let issues = check_problematic_path(Path::new("/games/cool!/mc"));
        assert!(has_blocking_path_issue(&issues));
    }

    #[test]
    fn flags_non_ascii_as_warning_only() {
        let issues = check_problematic_path(Path::new("/games/我的世界/mc"));
        assert!(!has_blocking_path_issue(&issues));
        assert!(issues.iter().any(|i| i.severity == PathSeverity::Warning));
    }

    #[test]
    fn normalizes_dot_segments() {
        assert_eq!(normalize(Path::new("a/b/../c/./d")), PathBuf::from("a/c/d"));
        assert_eq!(path_depth(Path::new("a/b/../c")), 2);
    }

    #[test]
    fn subpath_detection() {
        assert!(is_subpath(Path::new("/root/a/b"), Path::new("/root")));
        assert!(!is_subpath(Path::new("/root/../etc"), Path::new("/root")));
    }

    #[test]
    fn atomic_write_roundtrip() {
        let dir = std::env::temp_dir().join("mc-core-fs-test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("a/b/c.json");
        write_atomic(&p, b"hello").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hello");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_siblings_sharing_stem_dont_collide() {
        // a.json / a.txt / ... once shared one temp name (`a.tmp-PID`) and could clobber
        // each other when written concurrently. Each must keep its own content.
        let dir = std::env::temp_dir().join("mc-core-fs-siblings");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exts = ["json", "txt", "cfg", "log", "dat"];
        std::thread::scope(|s| {
            for ext in exts {
                let dir = &dir;
                s.spawn(move || {
                    let p = dir.join(format!("a.{ext}"));
                    write_atomic(&p, ext.as_bytes()).unwrap();
                });
            }
        });
        for ext in exts {
            let p = dir.join(format!("a.{ext}"));
            assert_eq!(std::fs::read_to_string(&p).unwrap(), ext);
        }
        // No temp files left behind.
        let leftover: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftover.is_empty(), "temp files left behind: {leftover:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn safe_join_blocks_traversal() {
        let base = Path::new("/games/mc");
        assert_eq!(safe_join(base, "config/options.txt"), Some(PathBuf::from("/games/mc/config/options.txt")));
        assert_eq!(safe_join(base, "../../etc/passwd"), None);
    }

    #[test]
    fn resolve_segment_rejects_traversal_and_separators() {
        let dir = Path::new("/games/mc/mods");
        // A plain single-segment name resolves to dir/<name>.
        assert_eq!(resolve_segment(dir, "sodium.jar").unwrap(), PathBuf::from("/games/mc/mods/sodium.jar"));
        // Every escape shape is rejected.
        assert!(resolve_segment(dir, "../x").is_err(), "parent-escape must be rejected");
        assert!(resolve_segment(dir, "a/b").is_err(), "embedded separator must be rejected");
        assert!(resolve_segment(dir, "a\\b").is_err(), "backslash separator must be rejected");
        assert!(resolve_segment(dir, "..").is_err(), "'..' must be rejected");
        assert!(resolve_segment(dir, ".").is_err(), "'.' must be rejected");
        assert!(resolve_segment(dir, "").is_err(), "empty segment must be rejected");
    }

    #[test]
    fn is_safe_segment_classifies_names() {
        assert!(is_safe_segment("world1"));
        assert!(is_safe_segment("My Cool Mod.jar"));
        assert!(!is_safe_segment("../x"));
        assert!(!is_safe_segment("a/b"));
        assert!(!is_safe_segment("a\\b"));
        assert!(!is_safe_segment(".."));
        assert!(!is_safe_segment("."));
        assert!(!is_safe_segment(""));
    }

    #[test]
    fn share_file_links_or_copies() {
        let dir = std::env::temp_dir().join("mc-core-share-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.bin");
        std::fs::write(&src, b"data").unwrap();
        let dst = dir.join("store/dst.bin");
        let method = share_file(&src, &dst).unwrap();
        assert!(matches!(method, ShareMethod::HardLink | ShareMethod::Reflink | ShareMethod::Copy));
        assert_eq!(std::fs::read(&dst).unwrap(), b"data");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn override_folder_overlays() {
        let dir = std::env::temp_dir().join("mc-core-override-test");
        let _ = std::fs::remove_dir_all(&dir);
        let ov = dir.join("overrides/config");
        std::fs::create_dir_all(&ov).unwrap();
        std::fs::write(ov.join("a.cfg"), b"x").unwrap();
        let target = dir.join("instance");
        override_folder(&dir.join("overrides"), &target).unwrap();
        assert!(target.join("config/a.cfg").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trash_or_delete_removes_file_and_dir() {
        // Whether the host has a trash backend (delete moves it) or not (hard
        // fallback removes it), the resolved target must be gone afterwards —
        // and the dir/file branch must vanish a directory tree as well as a file.
        let base = std::env::temp_dir().join(format!("mc-core-trash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        // A regular file → remove_file path.
        let file = base.join("doomed.txt");
        std::fs::write(&file, b"bye").unwrap();
        assert!(file.exists());
        trash_or_delete(&file).unwrap();
        assert!(!file.exists(), "file should be gone (trash or hard delete)");

        // A directory with contents → remove_dir_all path.
        let dir = base.join("doomed-dir");
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested/inner.bin"), b"x").unwrap();
        assert!(dir.exists());
        trash_or_delete(&dir).unwrap();
        assert!(!dir.exists(), "directory tree should be gone");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolves_known_executable() {
        // `sh` exists on every unix; on Windows skip (cmd resolution differs).
        if cfg!(unix) {
            assert!(resolve_executable("sh").is_some());
        }
        assert!(resolve_executable("definitely-not-a-real-binary-xyz").is_none());
    }
