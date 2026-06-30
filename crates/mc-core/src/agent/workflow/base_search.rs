use super::*;

use crate::modplatform::dependency::VersionLookupCache;
use crate::modplatform::ProjectVersion;
use futures::StreamExt;

/// 同时在途的 provider 请求上限。取小值以并发省时又不至于猛打 provider 触发限流。
const PROVIDER_FANOUT: usize = 8;

/// 并发抓取的一条结果:候选下标 + 平台 + 项目 id + `list_versions` 结果(成功才回填缓存)。
type FetchedVersions = (usize, ProviderId, String, Result<Vec<ProjectVersion>>);

pub(super) async fn continue_after_base_pack_choice(
    llm: &AgentLlmClient,
    mut run: AgentRunSnapshot,
    selected: ApprovalOption,
) -> Result<AgentRunSnapshot> {
    if selected.id == "scratch:fallback" || selected.id == "confirm:scratch_fallback" {
        return continue_after_scratch_base_choice(llm, run).await;
    }

    let base = parse_selected_base_pack(&selected)?;
    if !base_pack_provider_supported_for_execution(base.provider) {
        let base_pack_payload = selected.payload.clone().unwrap_or_else(|| {
            serde_json::json!({
                "provider": provider_slug(base.provider),
                "project_id": base.project_id,
                "slug": base.slug,
                "title": base.title,
            })
        });
        return Ok(block_base_pack_planning(
            run,
            base_pack_payload,
            format!(
                "The agent executor currently supports only Modrinth .mrpack base packs; {} base packs are not executable yet.",
                provider_label(base.provider)
            ),
        ));
    }
    run.push_message(
        AgentMessageKind::User,
        format!("Selected base modpack: {} ({})", base.title, selected.id),
    );
    let from_phase = run.phase.clone();
    run.phase = AgentPhase::CustomizationPlanning;
    run.push_trace(format!("selected base modpack {}", selected.id));
    invalidate_downstream(
        &mut run,
        ChangedField::BasePack,
        format!("selected base pack {}", base.title),
        from_phase,
        None,
    );

    let requested = requested_compatibility_from_restrictions(run.restrictions.as_ref());
    let compatibility = infer_base_pack_compatibility(&base, &requested).await?;
    let planning_input = planning_context_input(&run);

    let mut base_pack_payload = selected
        .payload
        .clone()
        .ok_or_else(|| CoreError::other("selected base pack option has no payload"))?;
    attach_base_pack_resolution(&mut base_pack_payload, &base, &compatibility);

    let base_modlist = match fetch_base_modlist_cache(&mut run, &base_pack_payload).await {
        Ok(cache) => cache,
        Err(err) => {
            return Ok(block_base_pack_planning(
                run,
                base_pack_payload,
                format!("Could not read the base-pack modlist: {err}"),
            ));
        }
    };

    let result = run_customization_planning_loop(
        llm,
        &mut run,
        &planning_input,
        &base,
        &compatibility,
        &[],
        &base_modlist,
    )
    .await?;
    let validated = match result {
        CustomizationPlanningResult::Validated(validated) => validated,
        CustomizationPlanningResult::Blocked(blocked) => {
            return Ok(block_customization_planning(run, blocked));
        }
    };
    run.push_message(
        AgentMessageKind::Tool,
        format!(
            "customization planning produced {} validated installable files using cached {} base modlist",
            validated.extra_mods.len(),
            base_modlist.source_format
        ),
    );
    let (plan, approval) = customization_approval_with_validation(
        &planning_input,
        &base,
        &compatibility,
        base_pack_payload,
        validated.extra_mods,
        Some(validated.validation),
    );

    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.pending_approval = Some(approval);
    run.plan = Some(plan);
    run.push_trace("paused at customization confirmation gate");
    Ok(run)
}

async fn continue_after_scratch_base_choice(
    llm: &AgentLlmClient,
    mut run: AgentRunSnapshot,
) -> Result<AgentRunSnapshot> {
    let from_phase = run.phase.clone();
    run.push_message(
        AgentMessageKind::User,
        "Selected base modpack: Start from scratch",
    );
    run.phase = AgentPhase::CustomizationPlanning;
    run.pending_approval = None;
    run.plan = Some(scratch_build_plan(&run.user_prompt));
    run.push_trace("selected scratch base set");
    invalidate_downstream(
        &mut run,
        ChangedField::BasePack,
        "selected scratch base set",
        from_phase,
        None,
    );

    let requested = requested_compatibility_from_restrictions(run.restrictions.as_ref());
    let compatibility = TargetCompatibility {
        minecraft_version: requested.minecraft_version.clone(),
        loader: requested.loader.clone(),
        version_id: None,
        version_name: None,
        version_number: None,
        game_versions: requested.minecraft_version.iter().cloned().collect(),
        loaders: requested.loader.iter().cloned().collect(),
        primary_file: None,
        dependencies: Vec::new(),
    };
    let base = SelectedBasePack {
        provider: ProviderId::Modrinth,
        project_id: "scratch".to_string(),
        slug: "scratch".to_string(),
        title: "Start from scratch".to_string(),
        description: Some("Empty base set".to_string()),
    };
    let base_pack_payload = scratch_base_pack_payload();
    let base_modlist = BaseModlistCache {
        refs: Vec::new(),
        source_format: "scratch_empty".to_string(),
        fetch_count: 0,
    };
    let planning_input = planning_context_input(&run);
    let result = run_customization_planning_loop(
        llm,
        &mut run,
        &planning_input,
        &base,
        &compatibility,
        &[],
        &base_modlist,
    )
    .await?;
    let validated = match result {
        CustomizationPlanningResult::Validated(validated) => validated,
        CustomizationPlanningResult::Blocked(blocked) => {
            return Ok(block_customization_planning(run, blocked));
        }
    };
    run.push_message(
        AgentMessageKind::Tool,
        format!(
            "scratch mod planning produced {} validated installable files",
            validated.extra_mods.len()
        ),
    );
    let (plan, approval) = customization_approval_with_validation(
        &planning_input,
        &base,
        &compatibility,
        base_pack_payload,
        validated.extra_mods,
        Some(validated.validation),
    );
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.pending_approval = Some(approval);
    run.plan = Some(plan);
    run.push_trace("paused at scratch customization confirmation gate");
    Ok(run)
}

pub(super) fn block_base_pack_planning(
    mut run: AgentRunSnapshot,
    base_pack_payload: serde_json::Value,
    reason: String,
) -> AgentRunSnapshot {
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ChooseBasePackApproval;
    run.pending_approval = Some(base_pack_planning_blocked_approval(
        &run,
        base_pack_payload,
        &reason,
    ));
    run.push_message(
        AgentMessageKind::Tool,
        format!("base-pack planning blocked: {reason}"),
    );
    run.push_trace("base-pack planning blocked; returned to base-pack HITL gate");
    run
}

fn base_pack_planning_blocked_approval(
    run: &AgentRunSnapshot,
    base_pack_payload: serde_json::Value,
    reason: &str,
) -> ApprovalRequest {
    let title = optional_json_string(&base_pack_payload, "title")
        .unwrap_or_else(|| "Current base pack".to_string());
    let provider = optional_json_string(&base_pack_payload, "provider")
        .unwrap_or_else(|| "selected".to_string());
    let project_id = optional_json_string(&base_pack_payload, "project_id")
        .unwrap_or_else(|| "base_pack".to_string());
    ApprovalRequest {
        id: crate::agent::state::new_id("approval"),
        kind: ApprovalKind::ChooseBasePack,
        title: "Base-pack planning is blocked".to_string(),
        message: format!("Failed to read or parse the current base-pack modlist: {reason}. Choose another base pack or change the search requirements."),
        options: vec![ApprovalOption {
            id: format!("{provider}:{project_id}"),
            label: title,
            description: Some(format!("Current base pack is blocked: {reason}")),
            payload: Some(base_pack_payload),
        }],
        available_decisions: approval_decisions("Keep this base pack", "Search base packs again"),
        tools: vec![update_build_restrictions_tool_spec()],
        plan: Some(ModpackAgentPlan {
            objective: run.user_prompt.clone(),
            summary_markdown: format!("Base-pack planning is blocked: {reason}"),
            risks: vec!["Continuing with the current base pack may hit the same block again."
                .to_string()],
            planned_actions: vec![PlannedAction {
                id: "replan-base-pack".to_string(),
                label: "User revises base pack after planning block".to_string(),
                tool: "approval_gate".to_string(),
                args: serde_json::json!({ "kind": "choose_base_pack", "planning_blocked": true }),
                requires_approval: true,
            }],
            migration_notes: vec![],
        }),
    }
}

pub(super) async fn continue_to_base_pack_search(
    llm: &AgentLlmClient,
    mut run: AgentRunSnapshot,
) -> Result<AgentRunSnapshot> {
    run.phase = AgentPhase::BasePackSearch;
    let planning_input = planning_context_input(&run);
    let output = llm
        .prompt_typed::<SearchQueryOutput>(
            &[MAIN_AGENT_SYSTEM_PROMPT, SEARCH_QUERY_PROMPT],
            planning_input.clone(),
            300,
            0.1,
        )
        .await?;
    run.push_trace(format!("llm generated search queries via {}", llm.model()));

    let queries = output.into_queries("base modpack search")?;
    run.push_message(
        AgentMessageKind::Assistant,
        format!(
            "Searching existing modpacks as the base before adding requested mods. Queries: {}",
            queries.join(", ")
        ),
    );

    let requested = requested_compatibility_from_restrictions(run.restrictions.as_ref());
    let candidates = run_base_pack_search_loop(&mut run, &queries, &requested).await?;
    run.push_trace(format!(
        "search_modpacks returned {} candidates",
        candidates.len()
    ));
    run.push_message(
        AgentMessageKind::Tool,
        format!("search_modpacks returned {} candidates", candidates.len()),
    );

    let plan = selection_plan(&planning_input, &queries, &candidates);
    let approval = base_pack_selection_approval(&candidates, plan.clone());

    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ChooseBasePackApproval;
    run.pending_approval = Some(approval);
    run.plan = Some(plan);
    run.push_trace("paused at base modpack selection approval gate");
    Ok(run)
}

pub(super) async fn continue_after_base_pack_feedback(
    llm: &AgentLlmClient,
    mut run: AgentRunSnapshot,
    feedback: &str,
) -> Result<AgentRunSnapshot> {
    run.push_message(
        AgentMessageKind::User,
        format!("Changed base-pack search requirements: {feedback}"),
    );
    run.phase = AgentPhase::BasePackSearch;
    run.pending_approval = None;
    run.push_trace("received base-pack feedback; updating restrictions and replanning candidates");

    let current = run.restrictions.clone().unwrap_or_default();
    let generated = generate_restriction_update(
        llm,
        &run.user_prompt,
        &current,
        feedback,
        BuildRestrictionChangeSource::UserRevise,
    )
    .await?;
    let output = update_build_restrictions(
        Some(current.clone()),
        generated.input,
        BuildRestrictionChangeSource::UserRevise,
        feedback,
    )?;
    if let Some(changed) = changed_restriction_field(&current, &output.restrictions) {
        let patch = output
            .restrictions
            .history
            .last()
            .map(|change| change.patch.clone());
        invalidate_downstream(
            &mut run,
            changed,
            format!("base-pack feedback changed target: {feedback}"),
            AgentPhase::ChooseBasePackApproval,
            patch,
        );
    }
    run.restrictions = Some(output.restrictions);
    run.push_trace(format!(
        "llm generated build restriction update via {}",
        generated.model
    ));

    let planning_input = planning_context_input(&run);
    let revised_prompt = format!(
        "{planning_input}\n\nBase-pack revision feedback: {feedback}\n\nSearch again for base modpack candidates that reflect the feedback."
    );
    let output = llm
        .prompt_typed::<SearchQueryOutput>(
            &[MAIN_AGENT_SYSTEM_PROMPT, SEARCH_QUERY_PROMPT],
            revised_prompt,
            300,
            0.1,
        )
        .await?;
    run.push_trace(format!(
        "llm regenerated base search queries via {}",
        llm.model()
    ));

    let queries = output.into_queries("base modpack search")?;
    run.push_message(
        AgentMessageKind::Assistant,
        format!(
            "Searching base packs again from feedback. Queries: {}",
            queries.join(", ")
        ),
    );

    let requested = requested_compatibility_from_restrictions(run.restrictions.as_ref());
    let candidates = run_base_pack_search_loop(&mut run, &queries, &requested).await?;
    run.push_trace(format!(
        "search_modpacks returned {} revised candidates",
        candidates.len()
    ));
    run.push_message(
        AgentMessageKind::Tool,
        format!("search_modpacks returned {} candidates", candidates.len()),
    );

    let plan = selection_plan(&planning_input, &queries, &candidates);
    let approval = base_pack_selection_approval(&candidates, plan.clone());
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ChooseBasePackApproval;
    run.pending_approval = Some(approval);
    run.plan = Some(plan);
    run.push_trace("paused at updated base modpack selection approval gate");
    Ok(run)
}

pub(super) async fn run_base_pack_search_loop(
    run: &mut AgentRunSnapshot,
    queries: &[String],
    requested: &RequestedCompatibility,
) -> Result<Vec<BasePackCandidate>> {
    let mut mode = BaseSearchMode::Strict;
    let mut best = Vec::new();
    // 跨 4 轮模式阶梯共享的 list_versions 缓存:同一个基础包在多轮里重复出现时,其版本只取一次。
    // 作用域仅限本搜索循环(返回即 drop),不会跨运行变陈旧。
    let mut version_cache = VersionLookupCache::new();

    for iteration in 1..=BASE_SEARCH_MAX_ITERATIONS {
        let started = Instant::now();
        let searched = search_base_modpacks(queries, requested, mode).await?;
        run.push_tool_trace(AgentToolTrace {
            event: "base-pack loop search_packs".into(),
            stage: AgentPhase::BasePackSearch,
            iteration,
            tool: "search_packs".into(),
            input: serde_json::json!({
                "queries": queries,
                "mode": base_search_mode_label(mode),
                "filters": {
                    "minecraft_version": requested.minecraft_version.clone(),
                    "loader": requested.loader.clone(),
                }
            }),
            output: serde_json::json!({ "count": searched.len() }),
            duration_ms: started.elapsed().as_millis(),
            status: "ok".into(),
        });

        let started = Instant::now();
        let filtered =
            filter_base_packs_by_restrictions(searched, requested, &mut version_cache).await?;
        run.push_tool_trace(AgentToolTrace {
            event: "base-pack loop filter_by_restrictions".into(),
            stage: AgentPhase::BasePackSearch,
            iteration,
            tool: "filter_by_restrictions".into(),
            input: serde_json::json!({
                "minecraft_version": requested.minecraft_version.clone(),
                "loader": requested.loader.clone(),
            }),
            output: serde_json::json!({ "count": filtered.len() }),
            duration_ms: started.elapsed().as_millis(),
            status: "ok".into(),
        });

        let started = Instant::now();
        let ranked = rank_base_packs(filtered);
        run.push_tool_trace(AgentToolTrace {
            event: "base-pack loop rank_packs".into(),
            stage: AgentPhase::BasePackSearch,
            iteration,
            tool: "rank_packs".into(),
            input: serde_json::json!({ "input_count": ranked.len() }),
            output: serde_json::json!({
                "count": ranked.len(),
                "top": ranked.iter().take(3).map(|c| c.hit.title.clone()).collect::<Vec<_>>()
            }),
            duration_ms: started.elapsed().as_millis(),
            status: "ok".into(),
        });

        if !ranked.is_empty() {
            best = ranked.clone();
        }
        if base_search_has_acceptable_count(ranked.len()) {
            return Ok(ranked
                .into_iter()
                .take(BASE_SEARCH_APPROVAL_LIMIT)
                .collect());
        }

        mode = next_base_search_mode(ranked.len());
    }

    Ok(best.into_iter().take(BASE_SEARCH_APPROVAL_LIMIT).collect())
}

async fn search_base_modpacks(
    queries: &[String],
    requested: &RequestedCompatibility,
    mode: BaseSearchMode,
) -> Result<Vec<BasePackCandidate>> {
    let registry = ProviderRegistry::with_defaults();
    let provider = registry
        .get(ProviderId::Modrinth)
        .ok_or_else(|| CoreError::other("Modrinth provider is not registered"))?;
    let provider_id = provider.id();

    // Build one SearchQuery per query text (the per-mode limit/sort/filters are identical to the
    // old sequential loop), then run the independent searches concurrently with bounded fan-out.
    let built: Vec<SearchQuery> = queries
        .iter()
        .map(|text| {
            let mut query = SearchQuery::new(text.clone(), ResourceKind::Modpack);
            query.limit = match mode {
                BaseSearchMode::Strict => 6,
                BaseSearchMode::Loose => 12,
                BaseSearchMode::Tight => 4,
            };
            query.sort = match mode {
                BaseSearchMode::Strict | BaseSearchMode::Loose => SortMethod::Relevance,
                BaseSearchMode::Tight => SortMethod::Downloads,
            };
            if !matches!(mode, BaseSearchMode::Loose) {
                query.game_version = requested.minecraft_version.clone();
                query.loader = requested.loader.clone();
            }
            query
        })
        .collect();

    // `buffered` keeps the results in input (query) order, so the cross-query dedup + cap below is
    // byte-for-byte identical to the sequential version. Collect into `Vec<Result<…>>` (not
    // `try_collect`) so that an error from a query the cap would have skipped is discarded, exactly
    // as before — the first error among the queries we actually consume still propagates via `?`.
    let results: Vec<Result<Vec<SearchHit>>> =
        futures::stream::iter(built.iter().map(|query| provider.search(query)))
            .buffered(PROVIDER_FANOUT)
            .collect()
            .await;

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (text, hits_result) in queries.iter().zip(results) {
        for hit in hits_result? {
            let key = format!("{provider_id:?}:{}", hit.id);
            if !seen.insert(key) {
                continue;
            }
            out.push(BasePackCandidate {
                provider: provider_id,
                hit,
                matched_query: text.clone(),
                resolved_target: None,
            });
            if out.len() >= BASE_SEARCH_MAX_CANDIDATES {
                return Ok(out);
            }
        }
    }

    Ok(out)
}

async fn filter_base_packs_by_restrictions(
    candidates: Vec<BasePackCandidate>,
    requested: &RequestedCompatibility,
    cache: &mut VersionLookupCache,
) -> Result<Vec<BasePackCandidate>> {
    let candidates = candidates
        .into_iter()
        .filter(|candidate| base_pack_provider_supported_for_execution(candidate.provider))
        .collect::<Vec<_>>();
    if requested.minecraft_version.is_none() && requested.loader.is_none() {
        return Ok(candidates);
    }

    let registry = ProviderRegistry::with_defaults();
    let mc = requested.minecraft_version.as_deref();
    let loader = requested.loader.as_deref();

    // Resolve each candidate's version list, reusing the run cache (cross-iteration hits) and
    // fetching the cache-misses concurrently with bounded fan-out. Output stays strictly in input
    // order, so the "first compatible version wins" pick and error propagation are unchanged.
    let mut fetch_futs = Vec::new();
    for (idx, candidate) in candidates.iter().enumerate() {
        if cache
            .get_cloned(candidate.provider, &candidate.hit.id, mc, loader)
            .is_some()
        {
            continue; // served from cache during assembly below
        }
        let provider_id = candidate.provider;
        if let Some(provider) = registry.get(provider_id) {
            let project_id = candidate.hit.id.clone();
            // `mc` / `loader` are `Option<&str>` into `requested`, which outlives every fetch, so
            // capture them by copy rather than allocating per-candidate owned strings.
            fetch_futs.push(async move {
                let versions = provider.list_versions(&project_id, mc, loader).await;
                (idx, provider_id, project_id, versions)
            });
        }
        // No registered provider → leave it a miss with no fetch; the candidate is skipped below,
        // exactly like the old `let Some(provider) = … else { continue }`.
    }

    let fetched: Vec<FetchedVersions> = futures::stream::iter(fetch_futs)
        .buffer_unordered(PROVIDER_FANOUT)
        .collect()
        .await;

    // Store successes into the cache; stash per-index results for ordered assembly.
    let mut fetched_by_idx: HashMap<usize, Result<Vec<ProjectVersion>>> = HashMap::new();
    for (idx, provider_id, project_id, versions) in fetched {
        if let Ok(ref v) = versions {
            cache.store(provider_id, &project_id, mc, loader, v.clone());
        }
        fetched_by_idx.insert(idx, versions);
    }

    let mut out = Vec::new();
    for (idx, candidate) in candidates.into_iter().enumerate() {
        let versions = if let Some(hit) = cache.get_cloned(candidate.provider, &candidate.hit.id, mc, loader) {
            hit
        } else if let Some(res) = fetched_by_idx.remove(&idx) {
            res? // first error in input order propagates, matching the sequential `?`
        } else {
            continue; // provider missing → skip this candidate
        };
        if let Some(version) = versions.first() {
            let mut candidate = candidate;
            candidate.resolved_target = Some(target_compatibility_from_version(version, requested));
            out.push(candidate);
        }
    }
    Ok(out)
}

pub(super) fn base_pack_provider_supported_for_execution(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::Modrinth)
}

// Per-rank weight for the cross-query Modrinth relevance order (discovery rank).
const BASE_RELEVANCE_WEIGHT: i64 = 2;
// Per-tier weight for log10-scaled download popularity.
const BASE_POPULARITY_WEIGHT: i64 = 3;
// Bonus when the matched query directly echoes the candidate title.
const BASE_TITLE_MATCH_BONUS: i64 = 3;

pub(super) fn rank_base_packs(candidates: Vec<BasePackCandidate>) -> Vec<BasePackCandidate> {
    // Capture each candidate's discovery order (its cross-query Modrinth relevance
    // rank, deduped to the first surfacing query) before sorting, then rank by
    // match quality + popularity. Archive size is demoted to a soft signal: we
    // still sink genuinely oversized (> cap, unexecutable) packs to the bottom and
    // break score ties toward smaller archives, but a merely-larger size never
    // buries a popular, directly-matching pack behind a tiny obscure one.
    let mut indexed: Vec<(usize, BasePackCandidate)> =
        candidates.into_iter().enumerate().collect();
    indexed.sort_by(|(a_rank, a), (b_rank, b)| {
        base_archive_oversized(a)
            .cmp(&base_archive_oversized(b))
            .then_with(|| base_match_score(*b_rank, b).cmp(&base_match_score(*a_rank, a)))
            .then_with(|| {
                base_archive_size(a)
                    .unwrap_or(u64::MAX)
                    .cmp(&base_archive_size(b).unwrap_or(u64::MAX))
            })
            .then_with(|| a_rank.cmp(b_rank))
            .then_with(|| a.hit.title.cmp(&b.hit.title))
    });
    indexed
        .into_iter()
        .map(|(_, candidate)| candidate)
        .collect()
}

fn base_match_score(discovery_rank: usize, candidate: &BasePackCandidate) -> i64 {
    let relevance = (BASE_SEARCH_MAX_CANDIDATES as i64
        - discovery_rank.min(BASE_SEARCH_MAX_CANDIDATES) as i64)
        * BASE_RELEVANCE_WEIGHT;
    let popularity = base_popularity_tier(candidate.hit.downloads) * BASE_POPULARITY_WEIGHT;
    let matched = if base_matched_query_hits_title(candidate) {
        BASE_TITLE_MATCH_BONUS
    } else {
        0
    };
    relevance + popularity + matched
}

fn base_popularity_tier(downloads: u64) -> i64 {
    downloads
        .checked_ilog10()
        .map_or(0, |digits| i64::from(digits) + 1)
}

fn base_matched_query_hits_title(candidate: &BasePackCandidate) -> bool {
    let title = candidate.hit.title.to_lowercase();
    candidate
        .matched_query
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .any(|token| title.contains(&token.to_lowercase()))
}

fn target_compatibility_from_version(
    version: &crate::modplatform::ProjectVersion,
    requested: &RequestedCompatibility,
) -> TargetCompatibility {
    TargetCompatibility {
        minecraft_version: requested
            .minecraft_version
            .clone()
            .or_else(|| version.game_versions.first().cloned()),
        loader: requested
            .loader
            .clone()
            .or_else(|| version.loaders.first().cloned()),
        version_id: Some(version.id.clone()),
        version_name: Some(version.name.clone()),
        version_number: Some(version.version_number.clone()),
        game_versions: version.game_versions.clone(),
        loaders: version.loaders.clone(),
        primary_file: version.primary_file().cloned(),
        dependencies: version.dependencies.clone(),
    }
}

fn base_archive_oversized(candidate: &BasePackCandidate) -> bool {
    // Only demote packs we can prove exceed the executable cap; unknown sizes are
    // not penalised here so the unconstrained case stays purely relevance-driven.
    base_archive_size(candidate).is_some_and(|size| size > MAX_BASE_ARCHIVE_BYTES as u64)
}

fn base_archive_size(candidate: &BasePackCandidate) -> Option<u64> {
    candidate
        .resolved_target
        .as_ref()
        .and_then(|target| target.primary_file.as_ref())
        .and_then(|file| file.size)
}

fn base_search_mode_label(mode: BaseSearchMode) -> &'static str {
    match mode {
        BaseSearchMode::Strict => "strict",
        BaseSearchMode::Loose => "loose",
        BaseSearchMode::Tight => "tight",
    }
}

pub(super) fn base_search_has_acceptable_count(count: usize) -> bool {
    (BASE_SEARCH_MIN_CANDIDATES..=BASE_SEARCH_MAX_CANDIDATES).contains(&count)
}

pub(super) fn next_base_search_mode(count: usize) -> BaseSearchMode {
    if count < BASE_SEARCH_MIN_CANDIDATES {
        BaseSearchMode::Loose
    } else {
        BaseSearchMode::Tight
    }
}

#[cfg(test)]
mod ranking_tests {
    use super::*;

    fn ranking_candidate(
        title: &str,
        downloads: u64,
        archive_size: Option<u64>,
        query: &str,
    ) -> BasePackCandidate {
        let resolved_target = archive_size.map(|size| TargetCompatibility {
            minecraft_version: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
            version_id: Some(format!("{title}-version")),
            version_name: Some(format!("{title} Version")),
            version_number: Some("1.0.0".to_string()),
            game_versions: vec!["1.20.1".to_string()],
            loaders: vec!["fabric".to_string()],
            primary_file: Some(VersionFile {
                url: format!("https://example.test/{title}.mrpack"),
                filename: format!("{title}.mrpack"),
                sha1: None,
                sha512: None,
                size: Some(size),
                primary: true,
                client_side: ProjectSideSupport::Unknown,
                server_side: ProjectSideSupport::Unknown,
            }),
            dependencies: Vec::new(),
        });
        BasePackCandidate {
            provider: ProviderId::Modrinth,
            hit: SearchHit {
                id: title.to_ascii_lowercase().replace(' ', "-"),
                slug: title.to_ascii_lowercase().replace(' ', "-"),
                title: title.to_string(),
                description: "test base pack".to_string(),
                author: "author".to_string(),
                downloads,
                icon_url: None,
                gallery_url: None,
                categories: Vec::new(),
                client_side: ProjectSideSupport::Unknown,
                server_side: ProjectSideSupport::Unknown,
            },
            matched_query: query.to_string(),
            resolved_target,
        }
    }

    #[test]
    fn popular_match_outranks_tiny_low_download_pack() {
        // The exact regression: both packs are well under the archive cap, the
        // tiny one is far smaller, yet the popular directly-matching pack
        // (discovered first, vastly more downloads) must rank first. Under the old
        // size-ascending comparator the tiny pack would have buried it.
        let popular = ranking_candidate(
            "Adventure Plus",
            500_000,
            Some(40 * 1024 * 1024),
            "fabric adventure",
        );
        let tiny = ranking_candidate("Tiny Obscure", 120, Some(256 * 1024), "fabric adventure");

        let ranked = rank_base_packs(vec![popular, tiny]);

        assert_eq!(ranked[0].hit.title, "Adventure Plus");
        assert_eq!(ranked[1].hit.title, "Tiny Obscure");
    }

    #[test]
    fn big_download_gap_overrides_a_single_relevance_rank() {
        // Popularity genuinely combines with discovery order: the hugely popular
        // pack discovered second still beats a low-download pack discovered first.
        let obscure_first = ranking_candidate("Obscure First", 90, Some(2 * 1024 * 1024), "fabric");
        let popular_second =
            ranking_candidate("Popular Second", 800_000, Some(2 * 1024 * 1024), "fabric");

        let ranked = rank_base_packs(vec![obscure_first, popular_second]);

        assert_eq!(ranked[0].hit.title, "Popular Second");
    }

    #[test]
    fn oversized_pack_is_demoted_below_executable_one() {
        // Keep the > cap (unexecutable) concern as a hard demotion even when the
        // oversized pack is more popular.
        let huge = ranking_candidate(
            "Huge Popular Pack",
            1_000_000,
            Some(MAX_BASE_ARCHIVE_BYTES as u64 + 1),
            "fabric",
        );
        let small = ranking_candidate("Small Pack", 500, Some(1024 * 1024), "fabric");

        let ranked = rank_base_packs(vec![huge, small]);

        assert_eq!(ranked[0].hit.title, "Small Pack");
        assert_eq!(ranked[1].hit.title, "Huge Popular Pack");
    }

    #[test]
    fn unconstrained_unknown_sizes_rank_by_relevance_then_downloads() {
        // No resolved_target → all sizes unknown → ranking must not no-op; it
        // falls back to discovery order + downloads rather than degrading.
        let first = ranking_candidate("First Found", 1_000, None, "fabric");
        let second = ranking_candidate("Second Found", 800, None, "fabric");

        let ranked = rank_base_packs(vec![first, second]);

        assert_eq!(ranked[0].hit.title, "First Found");
        assert_eq!(ranked[1].hit.title, "Second Found");
    }
}
