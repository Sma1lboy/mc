    use super::*;
    use crate::download::checksum::sha1_file;
    use std::fs;
    use std::path::PathBuf;

    struct TempInst {
        root: PathBuf,
        inst: Instance,
    }

    impl TempInst {
        fn new(tag: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("mc-core-realm-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let inst = Instance::new("1.20.1", root.clone());
            fs::create_dir_all(inst.mods_dir()).unwrap();
            Self { root, inst }
        }

        /// Write a top-level mod jar and return its sha1.
        fn put_mod(&self, file_name: &str, bytes: &[u8]) -> String {
            self.put_file(&format!("mods/{file_name}"), bytes)
        }

        /// Write a file at `rel` (relative to the instance dir) and return its sha1.
        fn put_file(&self, rel: &str, bytes: &[u8]) -> String {
            let p = self.inst.dir().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, bytes).unwrap();
            sha1_file(&p).unwrap()
        }
    }

    impl Drop for TempInst {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn url_file(path: &str, sha1: Option<String>) -> RealmFile {
        RealmFile {
            path: path.into(),
            sha1,
            sha512: None,
            size: None,
            url: Some("https://cdn.example/x.jar".into()),
            source: Some("modrinth".into()),
        }
    }

    /// Write a sync ledger claiming the given paths were installed by the syncer.
    fn put_ledger(t: &TempInst, version: i32, installed: &[&str]) {
        let state = SyncState {
            version,
            installed: installed.iter().map(|s| s.to_string()).collect(),
        };
        fs::write(t.inst.dir().join(SYNC_STATE_FILE), serde_json::to_vec(&state).unwrap()).unwrap();
    }

    #[test]
    fn plan_skips_present_matching_downloads_missing_and_flags_stale_and_manual() {
        let t = TempInst::new("plan");
        // present + correct hash → must be skipped
        let have_sha1 = t.put_mod("present.jar", b"already-here");
        // installed by a previous sync, dropped by this manifest → stale (remove)
        t.put_mod("extra.jar", b"previously-synced");
        put_ledger(&t, 6, &["mods/present.jar", "mods/extra.jar"]);

        let manifest = RealmManifest {
            mc_version: Some("1.20.1".into()),
            loader: Some("fabric".into()),
            loader_version: None,
            overrides: None,
            source: None,
            version: 7,
            files: vec![
                url_file("mods/present.jar", Some(have_sha1)), // matches → skip
                url_file("mods/missing.jar", Some("deadbeef".into())), // not on disk → download
                RealmFile {
                    path: "mods/custom.jar".into(),
                    sha1: Some("abc".into()),
                    sha512: None,
                    size: None,
                    url: None, // no url → manual
                    source: Some("manual".into()),
                },
            ],
        };

        let plan = plan_sync(&t.inst, &manifest);
        assert_eq!(plan.version, 7);
        assert_eq!(plan.download.len(), 1, "only the missing url file");
        assert_eq!(plan.download[0].path, "mods/missing.jar");
        assert_eq!(plan.manual.len(), 1);
        assert_eq!(plan.manual[0].path, "mods/custom.jar");
        // `extra.jar` is stale; `present.jar` and the manual `custom.jar` are not.
        assert_eq!(plan.remove, vec!["mods/extra.jar".to_string()]);
        // Everything url-carrying is ledger-managed after apply.
        assert_eq!(plan.managed, vec!["mods/present.jar".to_string(), "mods/missing.jar".to_string()]);
        assert!(!plan.is_up_to_date());
    }

    #[test]
    fn plan_never_removes_files_the_member_added_themselves() {
        let t = TempInst::new("useradd");
        // A local-only mod with NO ledger entry — user-added; must never be removed.
        t.put_mod("my-minimap.jar", b"user-added");
        let manifest = RealmManifest {
            files: vec![url_file("mods/shared.jar", Some("deadbeef".into()))],
            version: 3,
            ..Default::default()
        };
        let plan = plan_sync(&t.inst, &manifest);
        assert!(plan.remove.is_empty(), "user-added files are not the syncer's to delete");
        assert_eq!(plan.download.len(), 1);
    }

    #[test]
    fn plan_removes_the_disabled_twin_of_a_dropped_synced_mod() {
        let t = TempInst::new("disabled");
        // Ledger says the syncer installed dropped.jar; the member disabled it.
        t.put_mod("dropped.jar.disabled", b"was-synced");
        put_ledger(&t, 1, &["mods/dropped.jar"]);
        let plan = plan_sync(&t.inst, &RealmManifest { version: 2, ..Default::default() });
        assert_eq!(plan.remove, vec!["mods/dropped.jar.disabled".to_string()]);
    }

    #[test]
    fn plan_ignores_ledger_entries_already_gone_from_disk() {
        let t = TempInst::new("gone");
        put_ledger(&t, 1, &["mods/vanished.jar"]);
        let plan = plan_sync(&t.inst, &RealmManifest { version: 2, ..Default::default() });
        assert!(plan.remove.is_empty());
        assert!(plan.is_up_to_date());
    }

    #[test]
    fn plan_covers_resourcepacks_and_shaders_not_just_mods() {
        let t = TempInst::new("multidir");
        // a present, matching resourcepack → skipped; a missing shader → download;
        // a ledger-tracked, manifest-dropped resourcepack → remove.
        let rp = t.put_file("resourcepacks/faithful.zip", b"rp-bytes");
        t.put_file("resourcepacks/stale-rp.zip", b"old-rp");
        put_ledger(&t, 1, &["resourcepacks/faithful.zip", "resourcepacks/stale-rp.zip"]);
        let manifest = RealmManifest {
            files: vec![
                url_file("resourcepacks/faithful.zip", Some(rp)),
                url_file("shaderpacks/complementary.zip", Some("deadbeef".into())),
            ],
            version: 1,
            ..Default::default()
        };
        let plan = plan_sync(&t.inst, &manifest);
        assert_eq!(plan.download.len(), 1);
        assert_eq!(plan.download[0].path, "shaderpacks/complementary.zip");
        assert_eq!(plan.remove, vec!["resourcepacks/stale-rp.zip".to_string()]);
    }

    #[test]
    fn plan_is_up_to_date_when_instance_matches_manifest() {
        let t = TempInst::new("uptodate");
        let h = t.put_mod("sodium.jar", b"sodium-bytes");
        let manifest = RealmManifest {
            files: vec![url_file("mods/sodium.jar", Some(h))],
            version: 1,
            ..Default::default()
        };
        let plan = plan_sync(&t.inst, &manifest);
        assert!(plan.download.is_empty());
        assert!(plan.remove.is_empty());
        assert!(plan.is_up_to_date());
    }

    #[test]
    fn plan_rejects_path_traversal_and_absolute_paths() {
        let t = TempInst::new("traversal");
        // A tampered ledger can't steer removals outside the managed dirs either.
        put_ledger(&t, 1, &["../../outside.txt", "/etc/hosts", "config/user.toml"]);
        let manifest = RealmManifest {
            files: vec![
                url_file("../../evil.sh", Some("x".into())),        // parent escape
                url_file("/etc/cron.d/evil", Some("x".into())),     // absolute
                url_file("mods/../../escape.jar", Some("x".into())), // escape via ..
                url_file("config/evil.toml", Some("x".into())),     // outside mods/
                url_file("mods/ok.jar", Some("deadbeef".into())),   // the only legit one
            ],
            version: 1,
            ..Default::default()
        };
        let plan = plan_sync(&t.inst, &manifest);
        // Only the legit, missing mods/ok.jar is scheduled; every escaping path dropped.
        assert_eq!(plan.download.len(), 1);
        assert_eq!(plan.download[0].path, "mods/ok.jar");
        assert!(plan.manual.is_empty());
        assert!(plan.remove.is_empty());
    }

    #[test]
    fn overrides_zip_roundtrips_into_a_fresh_instance() {
        let src = TempInst::new("ov-src");
        src.put_file("config/sodium-options.json", b"{\"fps\":\"max\"}");
        src.put_file("config/sub/extra.toml", b"x=1");
        let zip = build_overrides_zip(
            &src.inst,
            &["config/sodium-options.json".into(), "config/sub/extra.toml".into()],
        )
        .unwrap()
        .expect("non-empty zip");

        let dst = TempInst::new("ov-dst");
        let n = apply_overrides(&dst.inst, &zip).unwrap();
        assert_eq!(n, 2);
        assert_eq!(
            std::fs::read(dst.inst.dir().join("config/sodium-options.json")).unwrap(),
            b"{\"fps\":\"max\"}"
        );
        assert!(dst.inst.dir().join("config/sub/extra.toml").exists());
    }

    #[test]
    fn apply_overrides_rejects_zip_slip() {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        w.start_file("../escape.txt", o).unwrap();
        w.write_all(b"pwned").unwrap();
        let bytes = w.finish().unwrap().into_inner();

        let t = TempInst::new("ov-slip");
        let n = apply_overrides(&t.inst, &bytes).unwrap();
        assert_eq!(n, 0, "the traversal entry must be skipped");
        assert!(!t.inst.dir().parent().unwrap().join("escape.txt").exists());
    }

    #[test]
    fn plan_redownloads_on_hash_mismatch() {
        let t = TempInst::new("mismatch");
        t.put_mod("sodium.jar", b"OLD-bytes");
        let manifest = RealmManifest {
            files: vec![url_file("mods/sodium.jar", Some("0000000000000000000000000000000000000000".into()))],
            version: 2,
            ..Default::default()
        };
        let plan = plan_sync(&t.inst, &manifest);
        assert_eq!(plan.download.len(), 1, "hash mismatch forces re-download");
        // present-but-wrong file is still manifest-referenced → not stale.
        assert!(plan.remove.is_empty());
    }
