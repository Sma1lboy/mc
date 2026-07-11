    use super::*;
    use crate::modplatform::provider::ResourceProvider;
    use crate::modplatform::{
        ProjectSideSupport, ProjectVersion, ProviderCaps, SearchHit, SearchQuery, VersionFile,
    };
    use futures::future::BoxFuture;
    use std::collections::HashMap;
    use std::fs;

    struct TempRoot {
        path: PathBuf,
    }
    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("mc-core-export-engine-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// 一个**离线假 provider**:按 sha512 字符串预置一张「哈希 → ResolvedFile」映射,
    /// `resolve_by_hashes` 据此返回对齐结果;其它方法不被引擎调用,给最小实现。
    struct FakeProvider {
        caps: ProviderCaps,
        by_hash: HashMap<String, ResolvedFile>,
    }

    impl FakeProvider {
        fn new(by_hash: HashMap<String, ResolvedFile>) -> Self {
            Self {
                caps: ProviderCaps {
                    id: ProviderId::Modrinth,
                    readable_name: "FakeModrinth",
                    hash_algos: &[HashAlgo::Sha512, HashAlgo::Sha1],
                    needs_api_key: false,
                },
                by_hash,
            }
        }
    }

    impl ResourceProvider for FakeProvider {
        fn caps(&self) -> &ProviderCaps {
            &self.caps
        }
        fn search<'a>(&'a self, _q: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn get_project<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, Result<SearchHit>> {
            Box::pin(async { Err(CoreError::other("unused")) })
        }
        fn get_projects<'a>(&'a self, _ids: &'a [String]) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn list_versions<'a>(
            &'a self,
            _project_id: &'a str,
            _gv: Option<&'a str>,
            _loader: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<ProjectVersion>>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn resolve_by_hashes<'a>(
            &'a self,
            _algo: HashAlgo,
            hashes: &'a [String],
        ) -> BoxFuture<'a, Result<Vec<Option<ResolvedFile>>>> {
            let out: Vec<Option<ResolvedFile>> =
                hashes.iter().map(|h| self.by_hash.get(h).cloned()).collect();
            Box::pin(async move { Ok(out) })
        }
        fn get_files_bulk<'a>(
            &'a self,
            _refs: &'a [(String, String)],
        ) -> BoxFuture<'a, Result<Vec<ResolvedFile>>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    fn resolved_file(host: &str, sha512: &str) -> ResolvedFile {
        ResolvedFile {
            provider: ProviderId::Modrinth,
            project_id: "AABBCCDD".into(),
            version_id: "v1".into(),
            file: VersionFile {
                url: format!("https://{host}/data/AABBCCDD/sodium.jar"),
                filename: "sodium.jar".into(),
                sha1: Some("deadbeef".into()),
                sha512: Some(sha512.into()),
                size: Some(100),
                primary: true,
                client_side: ProjectSideSupport::Unknown,
                server_side: ProjectSideSupport::Unknown,
            },
            project_name: Some("Sodium".into()),
            project_slug: Some("sodium".into()),
            authors: vec!["jellysquid".into()],
        }
    }

    /// 一个最小目标:gate = `mods/*.jar`,sha512 反查,allow_remote 看 host 白名单。
    struct TestTarget {
        allowed_host: &'static str,
    }
    impl ExportTarget for TestTarget {
        fn id(&self) -> &'static str {
            "test"
        }
        fn output_extension(&self) -> &'static str {
            "zip"
        }
        fn provider(&self) -> Option<ProviderId> {
            Some(ProviderId::Modrinth)
        }
        fn hash_algo(&self) -> Option<HashAlgo> {
            Some(HashAlgo::Sha512)
        }
        fn accepts(&self, rel: &Path) -> bool {
            let s = rel.to_string_lossy();
            s.starts_with("mods/") && s.ends_with(".jar")
        }
        fn allow_remote(&self, r: &ResolvedFile) -> bool {
            r.file
                .url
                .split('/')
                .nth(2)
                .map(|h| h.ends_with(self.allowed_host))
                .unwrap_or(false)
        }
        fn write_index(
            &self,
            _input: &ExportInput<'_>,
            _set: &ClassifiedSet,
        ) -> Result<Vec<(String, Vec<u8>)>> {
            Ok(vec![("index.json".into(), b"{}".to_vec())])
        }
    }

    /// 用一个含已知 sha512 的真实文件 + 假 provider,验证 resolvable vs override 分类。
    #[tokio::test]
    async fn classifies_resolvable_vs_override_with_fake_provider() {
        let root = TempRoot::new("classify");
        let g = &root.path;
        fs::create_dir_all(g.join("mods")).unwrap();
        // 这个文件会被反查命中(host 在白名单)。
        fs::write(g.join("mods/sodium.jar"), b"SODIUM-BYTES").unwrap();
        // 这个文件反查命中但 host **不**在白名单 → 回落 override。
        fs::write(g.join("mods/badhost.jar"), b"BADHOST-BYTES").unwrap();
        // 这个文件反查不到 → override。
        fs::write(g.join("mods/local.jar"), b"LOCAL-ONLY").unwrap();
        // 非门控文件 → 直接 override。
        fs::create_dir_all(g.join("config")).unwrap();
        fs::write(g.join("config/opts.toml"), b"k=1").unwrap();

        // 预置反查表:按真实 sha512。
        let sha_sodium = crate::download::checksum::sha512_file(&g.join("mods/sodium.jar")).unwrap();
        let sha_bad = crate::download::checksum::sha512_file(&g.join("mods/badhost.jar")).unwrap();
        let mut table = HashMap::new();
        table.insert(sha_sodium.clone(), resolved_file("cdn.modrinth.com", &sha_sodium));
        table.insert(sha_bad.clone(), resolved_file("evil.example.com", &sha_bad));

        let reg = ProviderRegistry::new().with(Arc::new(FakeProvider::new(table)));
        let exporter = ModpackExporter::new(Arc::new(reg));
        let target = TestTarget { allowed_host: "modrinth.com" };

        let files = walk::walk_game_root(g, &[]).unwrap();
        let set = exporter
            .classify(&target, &files, &mut |_, _, _| {})
            .await
            .unwrap();

        // sodium → resolvable;badhost / local / config → override。
        let resolved_rels: Vec<String> = set
            .resolved
            .iter()
            .map(|(p, _)| p.to_string_lossy().to_string())
            .collect();
        assert_eq!(resolved_rels, vec!["mods/sodium.jar"], "仅白名单 host 命中者可远程引用");

        let override_rels: Vec<String> =
            set.overrides.iter().map(|p| p.to_string_lossy().to_string()).collect();
        assert_eq!(
            override_rels,
            vec![
                "config/opts.toml",
                "mods/badhost.jar",
                "mods/local.jar",
            ],
            "非白名单 / 未命中 / 非门控都进 override"
        );

        // resolved_keys() 即 zip 排除集。
        let keys = set.resolved_keys();
        assert!(keys.contains("mods/sodium.jar"));
        assert!(!keys.contains("mods/local.jar"));
    }

    /// 端到端导出:resolved 文件被排除出 overrides、override 文件在包内、索引注入归档根。
    #[tokio::test]
    async fn export_excludes_resolved_from_zip() {
        use std::io::Read;
        let root = TempRoot::new("e2e");
        let g = root.path.join("versions").join("1.20.1");
        fs::create_dir_all(g.join("mods")).unwrap();
        fs::write(g.join("mods/sodium.jar"), b"SODIUM").unwrap();
        fs::write(g.join("mods/local.jar"), b"LOCAL").unwrap();

        let sha = crate::download::checksum::sha512_file(&g.join("mods/sodium.jar")).unwrap();
        let mut table = HashMap::new();
        table.insert(sha.clone(), resolved_file("cdn.modrinth.com", &sha));
        let reg = ProviderRegistry::new().with(Arc::new(FakeProvider::new(table)));
        let exporter = ModpackExporter::new(Arc::new(reg));
        let target = TestTarget { allowed_host: "modrinth.com" };

        let input = ExportInput::new(&g, "My Pack", "1.20.1");
        let dest = exporter.export(&target, input, &mut |_, _, _| {}).await.unwrap();
        assert!(dest.is_file());

        let f = fs::File::open(&dest).unwrap();
        let mut archive = ::zip::ZipArchive::new(f).unwrap();
        // 索引注入。
        assert!(archive.by_name("index.json").is_ok());
        // resolved 不在 overrides。
        assert!(archive.by_name("overrides/mods/sodium.jar").is_err());
        // local 在 overrides。
        let mut e = archive.by_name("overrides/mods/local.jar").unwrap();
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"LOCAL");
    }
