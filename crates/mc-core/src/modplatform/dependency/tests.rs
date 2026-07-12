    use super::*;
    use crate::modplatform::ProjectSideSupport;
    use crate::error::CoreError;
    use crate::modplatform::provider::ResourceProvider;
    use crate::modplatform::{
        Dependency, HashAlgo, ProviderCaps, SearchHit, SearchQuery, VersionFile,
    };
    use futures::future::BoxFuture;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// 内存版 provider:只实现 `caps` 与 `list_versions`(依赖解析唯一用到的两个方法)。
    /// 其余 trait 方法返回空/错误即可,因为解析器不调用它们。
    struct FakeProvider {
        caps: ProviderCaps,
        /// project_id -> 该项目的版本列表(已按"最新在前"排好)。
        versions: HashMap<String, Vec<ProjectVersion>>,
    }

    impl FakeProvider {
        fn new(id: ProviderId, versions: HashMap<String, Vec<ProjectVersion>>) -> Self {
            Self {
                caps: ProviderCaps {
                    id,
                    readable_name: "fake",
                    hash_algos: &[HashAlgo::Sha1],
                    needs_api_key: false,
                },
                versions,
            }
        }
    }

    impl ResourceProvider for FakeProvider {
        fn caps(&self) -> &ProviderCaps {
            &self.caps
        }

        fn search<'a>(&'a self, _q: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_project<'a>(&'a self, _project_id: &'a str) -> BoxFuture<'a, Result<SearchHit>> {
            Box::pin(async move { Err(CoreError::other("FakeProvider::get_project unused")) })
        }

        fn get_projects<'a>(
            &'a self,
            _project_ids: &'a [String],
        ) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn list_versions<'a>(
            &'a self,
            project_id: &'a str,
            _game_version: Option<&'a str>,
            _loader: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<ProjectVersion>>> {
            let result = self.versions.get(project_id).cloned().unwrap_or_default();
            Box::pin(async move { Ok(result) })
        }

        fn resolve_by_hashes<'a>(
            &'a self,
            _algo: HashAlgo,
            _hashes: &'a [String],
        ) -> BoxFuture<'a, Result<Vec<Option<ResolvedFile>>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_files_bulk<'a>(
            &'a self,
            _refs: &'a [(String, String)],
        ) -> BoxFuture<'a, Result<Vec<ResolvedFile>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }
    }

    /// 造一个兼容 `1.20.1` + `fabric`、带可选 `deps` 的版本,主文件 url 取自 `id`。
    fn version_with_deps(id: &str, deps: Vec<Dependency>) -> ProjectVersion {
        ProjectVersion {
            id: format!("{id}-v1"),
            name: format!("{id} 1.0"),
            version_number: "1.0".into(),
            game_versions: vec!["1.20.1".into()],
            loaders: vec!["fabric".into()],
            files: vec![VersionFile {
                url: format!("https://example.invalid/{id}.jar"),
                filename: format!("{id}.jar"),
                primary: true,
                ..Default::default()
            }],
            dependencies: deps,
            client_side: ProjectSideSupport::Unknown,
            server_side: ProjectSideSupport::Unknown,
        }
    }

    fn required_on(project_id: &str) -> Dependency {
        Dependency {
            project_id: Some(project_id.into()),
            version_id: None,
            dependency_type: "required".into(),
        }
    }

    fn registry_with(versions: HashMap<String, Vec<ProjectVersion>>) -> ProviderRegistry {
        let provider: Arc<dyn ResourceProvider> =
            Arc::new(FakeProvider::new(ProviderId::Modrinth, versions));
        ProviderRegistry::new().with(provider)
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        futures::executor::block_on(f)
    }

    #[test]
    fn pick_best_prefers_mc_and_loader_match() {
        let versions = vec![
            // 只匹配 loader,不匹配 mc。
            ProjectVersion {
                id: "wrong-mc".into(),
                name: "x".into(),
                version_number: "1".into(),
                game_versions: vec!["1.19.2".into()],
                loaders: vec!["fabric".into()],
                files: vec![],
                dependencies: vec![],
                client_side: ProjectSideSupport::Unknown,
                server_side: ProjectSideSupport::Unknown,
            },
            // 完全匹配。
            ProjectVersion {
                id: "perfect".into(),
                name: "y".into(),
                version_number: "2".into(),
                game_versions: vec!["1.20.1".into()],
                loaders: vec!["fabric".into()],
                files: vec![],
                dependencies: vec![],
                client_side: ProjectSideSupport::Unknown,
                server_side: ProjectSideSupport::Unknown,
            },
        ];
        let best = pick_best_version(&versions, "1.20.1", "fabric").unwrap();
        assert_eq!(best.id, "perfect");
    }

    #[test]
    fn pick_best_falls_back_to_first_when_none_match() {
        let versions = vec![ProjectVersion {
            id: "only".into(),
            name: "x".into(),
            version_number: "1".into(),
            game_versions: vec!["1.7.10".into()],
            loaders: vec!["forge".into()],
            files: vec![],
            dependencies: vec![],
            client_side: ProjectSideSupport::Unknown,
            server_side: ProjectSideSupport::Unknown,
        }];
        // 既不匹配 mc 也不匹配 loader → 退化到首项。
        let best = pick_best_version(&versions, "1.20.1", "fabric").unwrap();
        assert_eq!(best.id, "only");
        assert!(pick_best_version(&[], "1.20.1", "fabric").is_none());
    }

    #[test]
    fn root_with_required_dep_installs_both() {
        // root 依赖 lib;两者都应进 to_install。
        let mut versions = HashMap::new();
        versions.insert("root".to_string(), vec![version_with_deps("root", vec![required_on("lib")])]);
        versions.insert("lib".to_string(), vec![version_with_deps("lib", vec![])]);
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "root")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        let ids: HashSet<&str> = res.to_install.iter().map(|r| r.project_id.as_str()).collect();
        assert!(ids.contains("root"), "root should be installed");
        assert!(ids.contains("lib"), "required dep should be installed");
        assert_eq!(res.to_install.len(), 2);
        assert!(res.unresolved.is_empty());
        assert!(res.incompatible.is_empty());
        assert!(res.satisfied.is_empty());
        // 主文件 url 正确传递。
        let lib = res.to_install.iter().find(|r| r.project_id == "lib").unwrap();
        assert_eq!(lib.file.url, "https://example.invalid/lib.jar");
        assert_eq!(lib.version_id, "lib-v1");
    }

    #[test]
    fn incompatible_dep_lands_in_incompatible() {
        let mut versions = HashMap::new();
        let root = version_with_deps(
            "root",
            vec![Dependency {
                project_id: Some("badmod".into()),
                version_id: None,
                dependency_type: "incompatible".into(),
            }],
        );
        versions.insert("root".to_string(), vec![root]);
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "root")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        assert_eq!(res.to_install.len(), 1); // 只有 root
        assert_eq!(res.incompatible.len(), 1);
        assert_eq!(res.incompatible[0].project_id, "badmod");
        // 冲突项不应被请求/安装(FakeProvider 也没有它的版本)。
        assert!(res.to_install.iter().all(|r| r.project_id != "badmod"));
    }

    #[test]
    fn already_installed_root_is_satisfied_not_installed() {
        let mut versions = HashMap::new();
        versions.insert("root".to_string(), vec![version_with_deps("root", vec![])]);
        let registry = registry_with(versions);

        let mut installed = HashSet::new();
        installed.insert(ModRef::new(ProviderId::Modrinth, "root").key());

        let roots = [ModRef::new(ProviderId::Modrinth, "root")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &installed,
        ))
        .unwrap();

        assert!(res.to_install.is_empty(), "already-installed root must not reinstall");
        assert_eq!(res.satisfied.len(), 1);
        assert_eq!(res.satisfied[0].project_id, "root");
    }

    #[test]
    fn already_installed_dep_short_circuits_recursion() {
        // root -> lib(required),但 lib 已装 → lib 进 satisfied、不递归其依赖。
        let mut versions = HashMap::new();
        versions.insert("root".to_string(), vec![version_with_deps("root", vec![required_on("lib")])]);
        versions.insert(
            "lib".to_string(),
            vec![version_with_deps("lib", vec![required_on("deep")])],
        );
        versions.insert("deep".to_string(), vec![version_with_deps("deep", vec![])]);
        let registry = registry_with(versions);

        let mut installed = HashSet::new();
        installed.insert(ModRef::new(ProviderId::Modrinth, "lib").key());

        let roots = [ModRef::new(ProviderId::Modrinth, "root")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &installed,
        ))
        .unwrap();

        let ids: HashSet<&str> = res.to_install.iter().map(|r| r.project_id.as_str()).collect();
        assert!(ids.contains("root"));
        // lib 已装 → 不装、且其依赖 deep 不被拉取。
        assert!(!ids.contains("lib"));
        assert!(!ids.contains("deep"));
        assert!(res.satisfied.iter().any(|m| m.project_id == "lib"));
    }

    #[test]
    fn missing_provider_marks_unresolved() {
        // 引用一个 registry 里不存在的 provider(CurseForge 未注册)。
        let versions = HashMap::new();
        let registry = registry_with(versions); // 只注册了 Modrinth

        let roots = [ModRef::new(ProviderId::CurseForge, "whatever")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        assert!(res.to_install.is_empty());
        assert_eq!(res.unresolved.len(), 1);
        assert_eq!(res.unresolved[0].provider, ProviderId::CurseForge);
    }

    #[test]
    fn project_without_compatible_version_is_unresolved() {
        // 项目存在但没有任何版本 → unresolved(不是错误)。
        let mut versions = HashMap::new();
        versions.insert("empty".to_string(), Vec::new());
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "empty")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        assert!(res.to_install.is_empty());
        assert_eq!(res.unresolved.len(), 1);
        assert_eq!(res.unresolved[0].project_id, "empty");
    }

    #[test]
    fn cycle_terminates_via_visited_set() {
        // a -> b -> a(环)。visited 去重保证每个只解析一次、循环终止。
        let mut versions = HashMap::new();
        versions.insert("a".to_string(), vec![version_with_deps("a", vec![required_on("b")])]);
        versions.insert("b".to_string(), vec![version_with_deps("b", vec![required_on("a")])]);
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "a")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        let ids: HashSet<&str> = res.to_install.iter().map(|r| r.project_id.as_str()).collect();
        assert!(ids.contains("a"));
        assert!(ids.contains("b"));
        assert_eq!(res.to_install.len(), 2, "each node resolved exactly once despite the cycle");
    }

    #[test]
    fn self_loop_terminates() {
        // a -> a(自环)。
        let mut versions = HashMap::new();
        versions.insert("a".to_string(), vec![version_with_deps("a", vec![required_on("a")])]);
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "a")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        assert_eq!(res.to_install.len(), 1);
        assert_eq!(res.to_install[0].project_id, "a");
    }

    #[test]
    fn duplicate_roots_dedup() {
        // 同一个根给两次,只应解析/安装一次。
        let mut versions = HashMap::new();
        versions.insert("root".to_string(), vec![version_with_deps("root", vec![])]);
        let registry = registry_with(versions);

        let roots = [
            ModRef::new(ProviderId::Modrinth, "root"),
            ModRef::new(ProviderId::Modrinth, "root"),
        ];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        assert_eq!(res.to_install.len(), 1);
    }

    #[test]
    fn optional_dep_is_skipped() {
        let mut versions = HashMap::new();
        let root = version_with_deps(
            "root",
            vec![Dependency {
                project_id: Some("nicetohave".into()),
                version_id: None,
                dependency_type: "optional".into(),
            }],
        );
        versions.insert("root".to_string(), vec![root]);
        versions.insert("nicetohave".to_string(), vec![version_with_deps("nicetohave", vec![])]);
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "root")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        // optional 不自动安装。
        assert_eq!(res.to_install.len(), 1);
        assert_eq!(res.to_install[0].project_id, "root");
        assert!(res.incompatible.is_empty());
    }

    #[test]
    fn modref_key_distinguishes_providers() {
        let m = ModRef::new(ProviderId::Modrinth, "abc");
        let c = ModRef::new(ProviderId::CurseForge, "abc");
        assert_ne!(m.key(), c.key());
        assert_eq!(m.key(), "modrinth:abc");
        assert_eq!(c.key(), "curseforge:abc");
    }

    /// `list_versions` 调用计数版 provider:每次调用 +1。前 `fail_first` 次返回错误(用于验证
    /// 错误不入缓存),其余返回配置好的版本列表。
    struct CountingProvider {
        caps: ProviderCaps,
        versions: HashMap<String, Vec<ProjectVersion>>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
        fail_first: usize,
    }

    impl CountingProvider {
        fn new(
            versions: HashMap<String, Vec<ProjectVersion>>,
            fail_first: usize,
            calls: Arc<std::sync::atomic::AtomicUsize>,
        ) -> Self {
            Self {
                caps: ProviderCaps {
                    id: ProviderId::Modrinth,
                    readable_name: "counting",
                    hash_algos: &[HashAlgo::Sha1],
                    needs_api_key: false,
                },
                versions,
                calls,
                fail_first,
            }
        }
    }

    impl ResourceProvider for CountingProvider {
        fn caps(&self) -> &ProviderCaps {
            &self.caps
        }

        fn search<'a>(&'a self, _q: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_project<'a>(&'a self, _project_id: &'a str) -> BoxFuture<'a, Result<SearchHit>> {
            Box::pin(async move { Err(CoreError::other("CountingProvider::get_project unused")) })
        }

        fn get_projects<'a>(
            &'a self,
            _project_ids: &'a [String],
        ) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn list_versions<'a>(
            &'a self,
            project_id: &'a str,
            _game_version: Option<&'a str>,
            _loader: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<ProjectVersion>>> {
            use std::sync::atomic::Ordering;
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.fail_first {
                return Box::pin(async move { Err(CoreError::other("transient list_versions error")) });
            }
            let result = self.versions.get(project_id).cloned().unwrap_or_default();
            Box::pin(async move { Ok(result) })
        }

        fn resolve_by_hashes<'a>(
            &'a self,
            _algo: HashAlgo,
            _hashes: &'a [String],
        ) -> BoxFuture<'a, Result<Vec<Option<ResolvedFile>>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_files_bulk<'a>(
            &'a self,
            _refs: &'a [(String, String)],
        ) -> BoxFuture<'a, Result<Vec<ResolvedFile>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }
    }

    #[test]
    fn version_cache_serves_second_lookup_without_provider_call() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut versions = HashMap::new();
        versions.insert("root".to_string(), vec![version_with_deps("root", vec![])]);
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn ResourceProvider> =
            Arc::new(CountingProvider::new(versions, 0, calls.clone()));

        let mut cache = VersionLookupCache::new();
        let first =
            run(cache.list_versions(&provider, "root", Some("1.20.1"), Some("fabric"))).unwrap();
        let second =
            run(cache.list_versions(&provider, "root", Some("1.20.1"), Some("fabric"))).unwrap();

        // 第二次必须命中缓存,不再打 provider。
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // 命中返回的版本与首次实打的结果一致(缓存只改怎么取,不改取到什么)。
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, second[0].id);
    }

    #[test]
    fn version_cache_keys_on_filters_and_does_not_cache_errors() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // (a) 不同 (game_version, loader) 是不同键,各打一次;重复键命中。
        let mut versions = HashMap::new();
        versions.insert("root".to_string(), vec![version_with_deps("root", vec![])]);
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn ResourceProvider> =
            Arc::new(CountingProvider::new(versions, 0, calls.clone()));
        let mut cache = VersionLookupCache::new();
        run(cache.list_versions(&provider, "root", Some("1.20.1"), Some("fabric"))).unwrap();
        run(cache.list_versions(&provider, "root", Some("1.19.2"), Some("fabric"))).unwrap();
        run(cache.list_versions(&provider, "root", Some("1.20.1"), Some("fabric"))).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2, "distinct filter keys miss, repeated key hits");

        // (b) 第一次返回错误的查询不入缓存:下次同键仍需重打 provider。
        let mut versions2 = HashMap::new();
        versions2.insert("flaky".to_string(), vec![version_with_deps("flaky", vec![])]);
        let calls2 = Arc::new(AtomicUsize::new(0));
        let flaky: Arc<dyn ResourceProvider> =
            Arc::new(CountingProvider::new(versions2, 1, calls2.clone()));
        let mut cache2 = VersionLookupCache::new();
        assert!(run(cache2.list_versions(&flaky, "flaky", Some("1.20.1"), Some("fabric"))).is_err());
        assert!(run(cache2.list_versions(&flaky, "flaky", Some("1.20.1"), Some("fabric"))).is_ok());
        // 两次都打了 provider(错误未缓存),且成功结果现已缓存。
        assert_eq!(calls2.load(Ordering::SeqCst), 2);
        assert!(run(cache2.list_versions(&flaky, "flaky", Some("1.20.1"), Some("fabric"))).is_ok());
        assert_eq!(calls2.load(Ordering::SeqCst), 2, "the successful result is now cached");
    }

    #[test]
    fn shared_cache_dedups_list_versions_across_resolve_calls() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // 两个根 a、b 各依赖同一个 lib;两次独立 resolve 共用一个缓存时,lib 只取一次。
        let mut versions = HashMap::new();
        versions.insert("a".to_string(), vec![version_with_deps("a", vec![required_on("lib")])]);
        versions.insert("b".to_string(), vec![version_with_deps("b", vec![required_on("lib")])]);
        versions.insert("lib".to_string(), vec![version_with_deps("lib", vec![])]);
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn ResourceProvider> =
            Arc::new(CountingProvider::new(versions, 0, calls.clone()));
        let registry = ProviderRegistry::new().with(provider);

        let mut cache = VersionLookupCache::new();
        let res_a = run(resolve_dependencies_with_cache(
            &registry,
            &[ModRef::new(ProviderId::Modrinth, "a")],
            "1.20.1",
            "fabric",
            &HashSet::new(),
            &mut cache,
        ))
        .unwrap();
        let res_b = run(resolve_dependencies_with_cache(
            &registry,
            &[ModRef::new(ProviderId::Modrinth, "b")],
            "1.20.1",
            "fabric",
            &HashSet::new(),
            &mut cache,
        ))
        .unwrap();

        // a + lib(首次 resolve,2 次) + b(第二次 resolve 中 lib 命中缓存,仅 1 次) = 3。
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        // 结果与不带缓存逐字节一致:两次解析各自装上根 + 共享依赖。
        let ids_a: HashSet<&str> = res_a.to_install.iter().map(|r| r.project_id.as_str()).collect();
        assert!(ids_a.contains("a") && ids_a.contains("lib"));
        let ids_b: HashSet<&str> = res_b.to_install.iter().map(|r| r.project_id.as_str()).collect();
        assert!(ids_b.contains("b") && ids_b.contains("lib"));
    }
