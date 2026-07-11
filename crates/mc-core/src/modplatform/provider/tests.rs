    use super::*;

    #[test]
    fn host_routing_maps_known_cdns() {
        assert_eq!(host_provider("cdn.modrinth.com"), Some(ProviderId::Modrinth));
        assert_eq!(host_provider("api.modrinth.com"), Some(ProviderId::Modrinth));
        assert_eq!(host_provider("edge.forgecdn.net"), Some(ProviderId::CurseForge));
        assert_eq!(host_provider("mediafilez.forgecdn.net"), Some(ProviderId::CurseForge));
        assert_eq!(host_provider("api.curseforge.com"), Some(ProviderId::CurseForge));
        assert_eq!(host_provider("example.com"), None);
    }

    #[test]
    fn resolve_cf_api_key_prefers_settings_and_guards_empty() {
        // 非空设置 key 永远最高优先(链首),与环境无关,且去空白。
        assert_eq!(resolve_cf_api_key(Some("  my-key  ")).as_deref(), Some("my-key"));
        // 空 / 全空白的设置 key 绝不被当成有效 key 原样返回(它要么回退到 env 链,
        // 要么 None,但永不等于那串空白)。
        assert_ne!(resolve_cf_api_key(Some("")).as_deref(), Some(""));
        assert_ne!(resolve_cf_api_key(Some("   ")).as_deref(), Some("   "));
    }

    #[test]
    fn keyed_registry_registers_curseforge_with_explicit_key() {
        // 给了显式 key → Modrinth + CurseForge 都在。
        let reg = ProviderRegistry::with_defaults_keyed(Some("explicit-key".to_string()));
        assert!(reg.get(ProviderId::Modrinth).is_some());
        assert!(reg.get(ProviderId::CurseForge).is_some());
    }

    #[test]
    fn keyed_registry_always_has_modrinth() {
        // 即便没有任何 CurseForge key,Modrinth 也必定在。
        // (CurseForge 是否在取决于环境里有没有 MC_CF_API_KEY / baked key,故只断言 Modrinth。)
        let reg = ProviderRegistry::with_defaults_keyed(None);
        assert!(reg.get(ProviderId::Modrinth).is_some());
    }

    // ---------------------------------------------------------------------
    // search_concurrent: an in-memory registry makes the concentrated
    // orchestration (ordering, dedup, the skip-error rule, both cap shapes)
    // directly testable — impossible when the logic was inline over
    // `with_defaults()`'s live Modrinth/CurseForge providers.
    // ---------------------------------------------------------------------

    use crate::modplatform::{ProjectSideSupport, ResourceKind};

    /// A provider whose `search` is scripted per query text (and can be told to error), so tests can
    /// drive `search_concurrent`'s ordering / dedup / cap / error semantics deterministically.
    struct SearchFakeProvider {
        caps: ProviderCaps,
        /// query.text -> hit ids to return (in order).
        hits: HashMap<String, Vec<String>>,
        /// query.text values whose search returns an error instead of hits.
        errors: HashSet<String>,
    }

    impl SearchFakeProvider {
        fn new(id: ProviderId) -> Self {
            Self {
                caps: ProviderCaps {
                    id,
                    readable_name: "search fake",
                    hash_algos: &[],
                    needs_api_key: false,
                },
                hits: HashMap::new(),
                errors: HashSet::new(),
            }
        }

        fn returning(mut self, query: &str, ids: &[&str]) -> Self {
            self.hits
                .insert(query.to_string(), ids.iter().map(|s| s.to_string()).collect());
            self
        }

        fn erroring(mut self, query: &str) -> Self {
            self.errors.insert(query.to_string());
            self
        }
    }

    impl ResourceProvider for SearchFakeProvider {
        fn caps(&self) -> &ProviderCaps {
            &self.caps
        }

        fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
            Box::pin(async move {
                if self.errors.contains(&q.text) {
                    return Err(CoreError::other(format!("search boom: {}", q.text)));
                }
                let ids = self.hits.get(&q.text).cloned().unwrap_or_default();
                Ok(ids.iter().map(|id| search_test_hit(id)).collect())
            })
        }

        fn get_project<'a>(&'a self, _project_id: &'a str) -> BoxFuture<'a, Result<SearchHit>> {
            Box::pin(async move { Err(CoreError::other("SearchFakeProvider::get_project unused")) })
        }

        fn get_projects<'a>(
            &'a self,
            _project_ids: &'a [String],
        ) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn list_versions<'a>(
            &'a self,
            _project_id: &'a str,
            _game_version: Option<&'a str>,
            _loader: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<ProjectVersion>>> {
            Box::pin(async move { Ok(Vec::new()) })
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

    fn search_test_hit(id: &str) -> SearchHit {
        SearchHit {
            id: id.to_string(),
            slug: id.to_string(),
            title: id.to_string(),
            description: String::new(),
            author: String::new(),
            downloads: 0,
            icon_url: None,
            gallery_url: None,
            categories: Vec::new(),
            client_side: ProjectSideSupport::Unknown,
            server_side: ProjectSideSupport::Unknown,
        }
    }

    fn text_query(text: &str) -> SearchQuery {
        SearchQuery::new(text, ResourceKind::Mod)
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        futures::executor::block_on(f)
    }

    /// Flatten matches to `(provider, query_index, hit id)` for order-sensitive assertions.
    fn keys(matches: &[SearchMatch]) -> Vec<(ProviderId, usize, String)> {
        matches
            .iter()
            .map(|m| (m.provider, m.query_index, m.hit.id.clone()))
            .collect()
    }

    #[test]
    fn search_concurrent_orders_query_major_then_provider() {
        let mr = SearchFakeProvider::new(ProviderId::Modrinth)
            .returning("q0", &["mr-a"])
            .returning("q1", &["mr-b"]);
        let cf = SearchFakeProvider::new(ProviderId::CurseForge)
            .returning("q0", &["cf-a"])
            .returning("q1", &["cf-b"]);
        let reg = ProviderRegistry::new()
            .with(Arc::new(mr))
            .with(Arc::new(cf));
        let policy = DedupCapPolicy {
            providers: ProviderTargets::Only(vec![ProviderId::Modrinth, ProviderId::CurseForge]),
            per_query_cap: None,
            total_cap: 100,
        };
        let queries = vec![text_query("q0"), text_query("q1")];
        let out = run(reg.search_concurrent(&queries, PROVIDER_FANOUT, policy)).unwrap();
        assert_eq!(
            keys(&out),
            vec![
                (ProviderId::Modrinth, 0, "mr-a".to_string()),
                (ProviderId::CurseForge, 0, "cf-a".to_string()),
                (ProviderId::Modrinth, 1, "mr-b".to_string()),
                (ProviderId::CurseForge, 1, "cf-b".to_string()),
            ]
        );
    }

    #[test]
    fn search_concurrent_dedups_by_provider_and_id() {
        // (Modrinth,"dup") seen under q0 is dropped when it reappears under q1; the same id "shared"
        // from a *different* provider is kept, proving the dedup key spans (provider, id).
        let mr = SearchFakeProvider::new(ProviderId::Modrinth)
            .returning("q0", &["dup", "shared"])
            .returning("q1", &["dup"]);
        let cf = SearchFakeProvider::new(ProviderId::CurseForge).returning("q0", &["shared"]);
        let reg = ProviderRegistry::new()
            .with(Arc::new(mr))
            .with(Arc::new(cf));
        let policy = DedupCapPolicy {
            providers: ProviderTargets::Only(vec![ProviderId::Modrinth, ProviderId::CurseForge]),
            per_query_cap: None,
            total_cap: 100,
        };
        let queries = vec![text_query("q0"), text_query("q1")];
        let out = run(reg.search_concurrent(&queries, PROVIDER_FANOUT, policy)).unwrap();
        assert_eq!(
            keys(&out),
            vec![
                (ProviderId::Modrinth, 0, "dup".to_string()),
                (ProviderId::Modrinth, 0, "shared".to_string()),
                (ProviderId::CurseForge, 0, "shared".to_string()),
            ]
        );
    }

    #[test]
    fn search_concurrent_total_cap_discards_skipped_errors() {
        // q0 alone fills total_cap=2; q1 would error but is never consumed → Ok. This is the exact
        // "collect Vec<Result>, not try_collect; cap-skipped errors are discarded" rule.
        let mr = SearchFakeProvider::new(ProviderId::Modrinth)
            .returning("q0", &["a", "b", "c"])
            .erroring("q1");
        let reg = ProviderRegistry::new().with(Arc::new(mr));
        let policy = DedupCapPolicy {
            providers: ProviderTargets::Only(vec![ProviderId::Modrinth]),
            per_query_cap: None,
            total_cap: 2,
        };
        let queries = vec![text_query("q0"), text_query("q1")];
        let out = run(reg.search_concurrent(&queries, PROVIDER_FANOUT, policy)).unwrap();
        assert_eq!(
            keys(&out),
            vec![
                (ProviderId::Modrinth, 0, "a".to_string()),
                (ProviderId::Modrinth, 0, "b".to_string()),
            ]
        );
    }

    #[test]
    fn search_concurrent_first_consumed_error_propagates() {
        // An error in a result we actually reach (before any cap) propagates via `?`.
        let mr = SearchFakeProvider::new(ProviderId::Modrinth).erroring("q0");
        let reg = ProviderRegistry::new().with(Arc::new(mr));
        let policy = DedupCapPolicy {
            providers: ProviderTargets::Only(vec![ProviderId::Modrinth]),
            per_query_cap: None,
            total_cap: 100,
        };
        let queries = vec![text_query("q0")];
        let err = run(reg.search_concurrent(&queries, PROVIDER_FANOUT, policy)).unwrap_err();
        assert!(err.to_string().contains("search boom: q0"), "{err}");
    }

    #[test]
    fn search_concurrent_per_query_cap_is_soft_across_providers() {
        // per_query_cap=2: Modrinth's a1,a2 reach the cap and break its stream, but CurseForge is
        // still consumed for the same query and contributes its first hit b1 → 3 for one query. This
        // is the exact legacy `break`-the-inner-loop semantics, NOT a hard per-query cap of 2.
        let mr = SearchFakeProvider::new(ProviderId::Modrinth).returning("q0", &["a1", "a2", "a3"]);
        let cf = SearchFakeProvider::new(ProviderId::CurseForge).returning("q0", &["b1", "b2"]);
        let reg = ProviderRegistry::new()
            .with(Arc::new(mr))
            .with(Arc::new(cf));
        let policy = DedupCapPolicy {
            providers: ProviderTargets::Only(vec![ProviderId::Modrinth, ProviderId::CurseForge]),
            per_query_cap: Some(2),
            total_cap: 100,
        };
        let queries = vec![text_query("q0")];
        let out = run(reg.search_concurrent(&queries, PROVIDER_FANOUT, policy)).unwrap();
        assert_eq!(
            keys(&out),
            vec![
                (ProviderId::Modrinth, 0, "a1".to_string()),
                (ProviderId::Modrinth, 0, "a2".to_string()),
                (ProviderId::CurseForge, 0, "b1".to_string()),
            ]
        );
    }

    #[test]
    fn search_concurrent_customization_shape_applies_both_caps() {
        // per_query_cap=2 + total_cap=3 over a single provider: q0 yields a,b (per-query cap), q1
        // yields d and hits the total cap, so e is never added.
        let mr = SearchFakeProvider::new(ProviderId::Modrinth)
            .returning("q0", &["a", "b", "c"])
            .returning("q1", &["d", "e"]);
        let reg = ProviderRegistry::new().with(Arc::new(mr));
        let policy = DedupCapPolicy {
            providers: ProviderTargets::All,
            per_query_cap: Some(2),
            total_cap: 3,
        };
        let queries = vec![text_query("q0"), text_query("q1")];
        let out = run(reg.search_concurrent(&queries, PROVIDER_FANOUT, policy)).unwrap();
        assert_eq!(
            keys(&out),
            vec![
                (ProviderId::Modrinth, 0, "a".to_string()),
                (ProviderId::Modrinth, 0, "b".to_string()),
                (ProviderId::Modrinth, 1, "d".to_string()),
            ]
        );
    }

    #[test]
    fn search_concurrent_all_targets_follow_registry_order() {
        let mr = SearchFakeProvider::new(ProviderId::Modrinth).returning("q0", &["m"]);
        let cf = SearchFakeProvider::new(ProviderId::CurseForge).returning("q0", &["c"]);
        let reg = ProviderRegistry::new()
            .with(Arc::new(mr))
            .with(Arc::new(cf));
        // `All` fans out in exactly `registry.all()` order. HashMap iteration is stable for this
        // unmutated registry but not insertion-ordered, so compare against the observed order.
        let order: Vec<ProviderId> = reg.all().map(|p| p.id()).collect();
        let policy = DedupCapPolicy {
            providers: ProviderTargets::All,
            per_query_cap: None,
            total_cap: 100,
        };
        let queries = vec![text_query("q0")];
        let out = run(reg.search_concurrent(&queries, PROVIDER_FANOUT, policy)).unwrap();
        let got: Vec<ProviderId> = out.iter().map(|m| m.provider).collect();
        assert_eq!(got, order);
    }

    #[test]
    fn search_concurrent_only_missing_provider_errors() {
        let reg =
            ProviderRegistry::new().with(Arc::new(SearchFakeProvider::new(ProviderId::Modrinth)));
        let policy = DedupCapPolicy {
            providers: ProviderTargets::Only(vec![ProviderId::CurseForge]),
            per_query_cap: None,
            total_cap: 100,
        };
        let queries = vec![text_query("q0")];
        assert!(run(reg.search_concurrent(&queries, PROVIDER_FANOUT, policy)).is_err());
    }
