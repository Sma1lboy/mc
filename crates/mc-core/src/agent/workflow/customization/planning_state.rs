use super::*;

#[derive(Debug, Clone, Default)]
pub(in crate::agent::workflow) struct AppliedModPlanStep {
    pub(in crate::agent::workflow) selected_project_ids: Vec<String>,
    pub(in crate::agent::workflow) blockers: Vec<serde_json::Value>,
}

pub(in crate::agent::workflow) fn initialize_mod_plan_state(
    target: &TargetCompatibility,
    base_modlist: &BaseModlistCache,
    restrictions: Option<&BuildRestrictions>,
) -> ModPlanState {
    let loader = target.loader.as_deref().unwrap_or("unknown");
    let baseline_refs = baseline_mod_refs(loader);
    let mut goals = vec![Goal {
        id: baseline_goal_id(loader),
        label: format!("{loader} baseline"),
        kind: GoalKind::Baseline,
        status: if baseline_refs.is_empty() {
            GoalStatus::Dropped
        } else {
            GoalStatus::Open
        },
    }];
    let mut pending_queries = Vec::new();

    if let Some(restrictions) = restrictions {
        for (idx, tag) in restrictions.feature_tags.iter().enumerate() {
            let label = tag.trim();
            if label.is_empty() {
                continue;
            }
            let id = stable_goal_id("theme", label, idx);
            goals.push(Goal {
                id: id.clone(),
                label: label.to_string(),
                kind: GoalKind::Theme,
                status: GoalStatus::Open,
            });
            pending_queries.push(GoalQuery {
                goal_id: id,
                query: label.to_string(),
            });
        }
    }

    ModPlanState {
        target: target.clone(),
        base_set: base_modlist
            .refs
            .iter()
            .map(|r| ResolvedMod {
                provider: provider_slug(r.provider).to_string(),
                project_id: r.project_id.clone(),
                slug: None,
                title: None,
                version_id: None,
                filename: None,
                source_ref: None,
                payload: serde_json::json!({
                    "provider": provider_slug(r.provider),
                    "project_id": r.project_id,
                }),
                provenance: ModProvenance::BaseSet,
                goal_id: None,
            })
            .collect(),
        goals,
        additions: Vec::new(),
        removals: Vec::new(),
        blocked: Vec::new(),
        round: 0,
        empty_candidate_rounds: 0,
        pending_queries,
        base_covered_goals: Vec::new(),
    }
}

pub(super) fn baseline_goal_id(loader: &str) -> String {
    format!("baseline:{}", loader.trim().to_ascii_lowercase())
}

pub(super) fn stable_goal_id(prefix: &str, label: &str, idx: usize) -> String {
    let slug = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        format!("{prefix}:{idx}")
    } else {
        format!("{prefix}:{slug}")
    }
}

pub(in crate::agent::workflow) fn baseline_mod_refs(loader: &str) -> Vec<ModRef> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "fabric" | "quilt" => vec![ModRef::new(ProviderId::Modrinth, "P7dR8mSH")],
        _ => Vec::new(),
    }
}

pub(super) fn baseline_goal_map(roots: &[ModRef], loader: &str) -> HashMap<String, String> {
    let goal_id = baseline_goal_id(loader);
    roots
        .iter()
        .map(|root| (root.key(), goal_id.clone()))
        .collect()
}

pub(super) fn has_open_goals(state: &ModPlanState) -> bool {
    state
        .goals
        .iter()
        .any(|goal| goal.status == GoalStatus::Open)
}

pub(super) fn has_open_theme_goals(state: &ModPlanState) -> bool {
    state
        .goals
        .iter()
        .any(|goal| goal.kind == GoalKind::Theme && goal.status == GoalStatus::Open)
}

pub(super) fn provider_id_from_slug(slug: &str) -> Option<ProviderId> {
    match slug.trim().to_ascii_lowercase().as_str() {
        "modrinth" => Some(ProviderId::Modrinth),
        "curseforge" => Some(ProviderId::CurseForge),
        _ => None,
    }
}

/// Best-effort batch fetch of each base-pack mod's metadata (title, slug,
/// categories, description) keyed by `provider:project_id`. Any provider error
/// (or an unregistered provider, e.g. CurseForge without an API key) simply
/// yields fewer entries; the caller falls back to ids-only behavior.
pub(super) async fn enrich_base_set_metadata(
    registry: &ProviderRegistry,
    base_set: &[ResolvedMod],
) -> HashMap<String, SearchHit> {
    const BATCH: usize = 100;
    let mut by_provider: HashMap<ProviderId, Vec<String>> = HashMap::new();
    let mut seen = HashSet::new();
    for entry in base_set {
        let Some(provider_id) = provider_id_from_slug(&entry.provider) else {
            continue;
        };
        let key = provider_project_key(&entry.provider, &entry.project_id);
        if !seen.insert(key) {
            continue;
        }
        by_provider
            .entry(provider_id)
            .or_default()
            .push(entry.project_id.clone());
    }

    let mut out = HashMap::new();
    for (provider_id, ids) in by_provider {
        let Some(provider) = registry.get(provider_id) else {
            continue;
        };
        for chunk in ids.chunks(BATCH) {
            let Ok(hits) = provider.get_projects(chunk).await else {
                continue;
            };
            for hit in hits {
                out.insert(
                    provider_project_key(provider_slug(provider_id), &hit.id),
                    hit,
                );
            }
        }
    }
    out
}

/// Fill enriched metadata onto the `base_set` entries (title/slug fields plus a
/// richer payload), without overwriting any value that is already present.
pub(super) fn apply_base_set_metadata(state: &mut ModPlanState, meta: &HashMap<String, SearchHit>) {
    for entry in state.base_set.iter_mut() {
        let key = provider_project_key(&entry.provider, &entry.project_id);
        let Some(hit) = meta.get(&key) else {
            continue;
        };
        if entry
            .title
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
            && !hit.title.trim().is_empty()
        {
            entry.title = Some(hit.title.trim().to_string());
        }
        if entry
            .slug
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
            && !hit.slug.trim().is_empty()
        {
            entry.slug = Some(hit.slug.trim().to_string());
        }
        if let Some(obj) = entry.payload.as_object_mut() {
            if !hit.title.trim().is_empty() {
                obj.insert("title".to_string(), serde_json::json!(hit.title));
            }
            if !hit.slug.trim().is_empty() {
                obj.insert("slug".to_string(), serde_json::json!(hit.slug));
            }
            if !hit.categories.is_empty() {
                obj.insert("categories".to_string(), serde_json::json!(hit.categories));
            }
            if !hit.description.trim().is_empty() {
                obj.insert(
                    "description".to_string(),
                    serde_json::json!(hit.description),
                );
            }
        }
    }
}

/// The base-pack modlist entries (title + categories) fed to the coverage
/// analysis. Entries with neither a title nor categories carry no signal and
/// are dropped; the list is capped to keep the prompt bounded for large packs.
pub(super) fn base_modlist_coverage_entries(state: &ModPlanState) -> Vec<serde_json::Value> {
    const MAX_ENTRIES: usize = 80;
    state
        .base_set
        .iter()
        .filter_map(|m| {
            let title = m.title.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let categories = m
                .payload
                .get("categories")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if title.is_none() && categories.is_empty() {
                return None;
            }
            Some(serde_json::json!({
                "project_id": m.project_id,
                "title": title,
                "categories": categories,
            }))
        })
        .take(MAX_ENTRIES)
        .collect()
}

pub(super) fn base_coverage_prompt_payload(
    user_prompt: &str,
    base: &SelectedBasePack,
    open_theme_goals: &[&Goal],
    base_mods: &[serde_json::Value],
) -> serde_json::Value {
    serde_json::json!({
        "user_prompt": user_prompt,
        "selected_base_pack": {
            "title": base.title.clone(),
            "description": base.description.clone(),
        },
        "theme_goals": open_theme_goals
            .iter()
            .map(|goal| serde_json::json!({ "id": goal.id, "label": goal.label }))
            .collect::<Vec<_>>(),
        "base_pack_modlist": base_mods,
    })
}

/// One LLM round-trip that asks which open theme goals the base pack already
/// covers, then marks those goals `Covered` (recording them in
/// `base_covered_goals`) and drops their pending search queries so the loop
/// never searches for them. Model-agnostic: a cheaper model swap just works.
pub(super) async fn analyze_base_pack_coverage(
    llm: &AgentLlmClient,
    run: &mut AgentRunSnapshot,
    user_prompt: &str,
    base: &SelectedBasePack,
    state: &mut ModPlanState,
) -> Result<()> {
    let open_theme_goals = state
        .goals
        .iter()
        .filter(|goal| goal.kind == GoalKind::Theme && goal.status == GoalStatus::Open)
        .cloned()
        .collect::<Vec<_>>();
    if open_theme_goals.is_empty() {
        return Ok(());
    }
    let base_mods = base_modlist_coverage_entries(state);
    if base_mods.is_empty() {
        return Ok(());
    }
    let goal_ids = open_theme_goals
        .iter()
        .map(|goal| goal.id.clone())
        .collect::<HashSet<_>>();

    let started = Instant::now();
    let payload = base_coverage_prompt_payload(
        user_prompt,
        base,
        &open_theme_goals.iter().collect::<Vec<_>>(),
        &base_mods,
    );
    let output = llm
        .prompt_text(
            &[
                MAIN_AGENT_SYSTEM_PROMPT,
                modpack_build_react_prompt(),
                BASE_COVERAGE_PROMPT,
            ],
            payload.to_string(),
            300,
            0.0,
        )
        .await?;
    let output = parse_base_coverage_response(&output)?;
    let covered = output.covered_goal_ids(&goal_ids);

    run.push_tool_trace(AgentToolTrace {
        event: "modplan reducer base_pack_coverage".into(),
        stage: AgentPhase::CustomizationPlanning,
        iteration: 0,
        tool: "analyze_base_pack_coverage".into(),
        input: serde_json::json!({
            "theme_goals": open_theme_goals
                .iter()
                .map(|goal| goal.id.clone())
                .collect::<Vec<_>>(),
            "base_modlist_count": base_mods.len(),
        }),
        output: serde_json::json!({
            "model": llm.model(),
            "covered_goal_ids": covered.clone(),
            "covered_goals": output.trace_payload(),
        }),
        duration_ms: started.elapsed().as_millis(),
        status: "ok".into(),
    });

    for goal_id in &covered {
        mark_goal_status(state, goal_id, GoalStatus::Covered);
        push_unique_string(&mut state.base_covered_goals, goal_id.to_string());
    }
    state
        .pending_queries
        .retain(|query| !covered.contains(&query.goal_id));
    Ok(())
}

/// Honest, schema-stable summary of which goals the base pack covered, for the
/// ConfirmCustomization validation. A zero-addition plan that covers goals reads
/// as "the base pack already covers your requirements", not as a failure.
pub(super) fn base_pack_coverage_payload(state: &ModPlanState) -> serde_json::Value {
    let covered_goals = state
        .goals
        .iter()
        .filter(|goal| state.base_covered_goals.contains(&goal.id))
        .map(|goal| serde_json::json!({ "goal_id": goal.id, "label": goal.label }))
        .collect::<Vec<_>>();
    let theme_added = state
        .goals
        .iter()
        .filter(|goal| {
            goal.kind == GoalKind::Theme
                && goal.status == GoalStatus::Covered
                && !state.base_covered_goals.contains(&goal.id)
        })
        .count();
    let covered_count = state.base_covered_goals.len();
    let summary = if covered_count == 0 {
        String::new()
    } else if theme_added == 0 {
        format!(
            "The selected base pack already covers your requested features ({covered_count} satisfied by base-pack mods); no extra mods are needed for them."
        )
    } else {
        format!(
            "The selected base pack already covers {covered_count} requested feature(s); {theme_added} extra mod(s) were planned for the rest."
        )
    };
    serde_json::json!({
        "covered_goal_ids": state.base_covered_goals.clone(),
        "covered_goals": covered_goals,
        "summary": summary,
    })
}

pub(super) fn open_goal_queries(state: &ModPlanState) -> Vec<GoalQuery> {
    state
        .goals
        .iter()
        .filter(|goal| goal.status == GoalStatus::Open && goal.kind == GoalKind::Theme)
        .map(|goal| GoalQuery {
            goal_id: goal.id.clone(),
            query: goal.label.clone(),
        })
        .collect()
}

pub(super) fn current_project_ids(state: &ModPlanState) -> HashSet<String> {
    let removed = state.removals.iter().cloned().collect::<HashSet<_>>();
    state
        .base_set
        .iter()
        .chain(state.additions.iter())
        .filter(|m| !removed.contains(&m.project_id))
        .map(|m| m.project_id.clone())
        .collect()
}

pub(super) fn installed_mod_keys(state: &ModPlanState) -> HashSet<String> {
    let removed = state.removals.iter().cloned().collect::<HashSet<_>>();
    state
        .base_set
        .iter()
        .chain(state.additions.iter())
        .filter(|m| !removed.contains(&m.project_id))
        .map(|m| provider_project_key(&m.provider, &m.project_id))
        .collect()
}

pub(super) fn provider_project_key(provider: &str, project_id: &str) -> String {
    format!("{}:{}", provider.trim().to_ascii_lowercase(), project_id)
}

pub(in crate::agent::workflow) fn prefilter_mod_candidates(
    candidates: Vec<ModCandidate>,
    state: &ModPlanState,
) -> Vec<ModCandidate> {
    let current = current_project_ids(state);
    let removed = state.removals.iter().cloned().collect::<HashSet<_>>();
    let blocked = state.blocked.iter().cloned().collect::<HashSet<_>>();
    candidates
        .into_iter()
        .filter(|candidate| {
            !current.contains(&candidate.hit.id)
                && !removed.contains(&candidate.hit.id)
                && !blocked.contains(&candidate.hit.id)
        })
        .collect()
}

/// Test-facing wrapper preserving the original `apply_mod_plan_step` signature (workflow/tests.rs
/// drives it directly). The production planning loop calls [`apply_mod_plan_step_cached`] with its
/// run-scoped cache; a one-shot cache here only memoizes within a single step's selections and
/// cannot change results (deterministic list_versions), so the two are behaviorally identical.
#[cfg(test)]
pub(in crate::agent::workflow) async fn apply_mod_plan_step(
    registry: &ProviderRegistry,
    state: &mut ModPlanState,
    candidates: &[ModCandidate],
    step: ModPlanStep,
    mc_version: &str,
    loader: &str,
) -> Result<AppliedModPlanStep> {
    let mut cache = VersionLookupCache::new();
    apply_mod_plan_step_cached(
        registry, state, candidates, step, mc_version, loader, &mut cache,
    )
    .await
}

/// Same as [`apply_mod_plan_step`] but reuses the caller's run-scoped [`VersionLookupCache`] so the
/// per-selection dependency walks share memoized version lookups across selections and rounds.
///
/// The per-selection loop stays **sequential on purpose**: each selection's `already_installed` set
/// and `current_project_ids` guard depend on the mutations (`state.additions` / `state.blocked`)
/// made by earlier selections in the same step, so the selections are not independent and parallel
/// execution would change which mods each resolution sees as installed — and thus the blocker /
/// goal attribution. The cache only memoizes the network `list_versions` (state-independent), so it
/// preserves results exactly while removing the repeated lookups.
pub(in crate::agent::workflow) async fn apply_mod_plan_step_cached(
    registry: &ProviderRegistry,
    state: &mut ModPlanState,
    candidates: &[ModCandidate],
    step: ModPlanStep,
    mc_version: &str,
    loader: &str,
    cache: &mut VersionLookupCache,
) -> Result<AppliedModPlanStep> {
    for project_id in step.removals {
        if !state.removals.contains(&project_id) {
            state.removals.push(project_id.clone());
        }
        state.additions.retain(|m| m.project_id != project_id);
    }
    state.pending_queries = step.next_queries;

    let candidate_by_project = candidates
        .iter()
        .map(|candidate| (candidate.hit.id.as_str(), candidate))
        .collect::<HashMap<_, _>>();
    let root_hits = candidates
        .iter()
        .map(|candidate| (mod_ref_from_candidate(candidate).key(), candidate))
        .collect::<HashMap<_, _>>();
    let mut selected_project_ids = Vec::new();
    let mut all_blockers = Vec::new();
    for selection in step.selections {
        let Some(candidate) = candidate_by_project.get(selection.project_id.as_str()) else {
            continue;
        };
        remove_project_id(&mut state.removals, &candidate.hit.id);
        if current_project_ids(state).contains(&candidate.hit.id) {
            continue;
        }
        let root = mod_ref_from_candidate(candidate);
        selected_project_ids.push(candidate.hit.id.clone());
        let roots = vec![root.clone()];
        let mut root_goal_by_key = HashMap::new();
        root_goal_by_key.insert(root.key(), selection.goal_id.clone());

        let installed = installed_mod_keys(state);
        let resolution = resolve_dependencies_with_cache(
            registry, &roots, mc_version, loader, &installed, cache,
        )
        .await?;
        let blockers = customization_blockers(&resolution);
        if !blockers.is_empty() {
            push_unique_string(&mut state.blocked, candidate.hit.id.clone());
            mark_goal_status(state, &selection.goal_id, GoalStatus::Open);
            all_blockers.extend(blockers);
            continue;
        }

        remove_project_id(&mut state.blocked, &candidate.hit.id);
        append_dependency_resolution(
            state,
            &resolution,
            &roots,
            &root_hits,
            &root_goal_by_key,
            ModProvenance::Selected,
        );
    }
    Ok(AppliedModPlanStep {
        selected_project_ids,
        blockers: all_blockers,
    })
}

pub(in crate::agent::workflow) fn append_dependency_resolution(
    state: &mut ModPlanState,
    resolution: &crate::modplatform::dependency::DepResolution,
    roots: &[ModRef],
    root_hits: &HashMap<String, &ModCandidate>,
    root_goal_by_key: &HashMap<String, String>,
    root_provenance: ModProvenance,
) {
    let root_keys = roots.iter().map(ModRef::key).collect::<HashSet<_>>();
    let mut current = current_project_ids(state);
    for resolved in &resolution.to_install {
        remove_project_id(&mut state.removals, &resolved.project_id);
        if current.contains(&resolved.project_id) {
            continue;
        }
        let key = ModRef::new(resolved.provider, resolved.project_id.clone()).key();
        let is_root = root_keys.contains(&key);
        let goal_id = if is_root {
            root_goal_by_key.get(&key).cloned()
        } else {
            Some(ensure_dependency_goal(state, &resolved.project_id))
        };
        if let Some(goal_id) = goal_id.as_deref() {
            mark_goal_status(state, goal_id, GoalStatus::Covered);
        }
        let payload = resolved_file_mod_payload(resolved, &root_keys, root_hits);
        if let Some(mod_entry) = resolved_mod_from_payload(
            payload,
            if is_root {
                root_provenance.clone()
            } else {
                ModProvenance::Dependency
            },
            goal_id,
        ) {
            current.insert(mod_entry.project_id.clone());
            state.additions.push(mod_entry);
        }
    }
}

pub(super) fn push_unique_string(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

pub(super) fn remove_project_id(values: &mut Vec<String>, project_id: &str) {
    values.retain(|existing| existing != project_id);
}

pub(super) fn ensure_dependency_goal(state: &mut ModPlanState, project_id: &str) -> String {
    let id = format!("dependency:{project_id}");
    if !state.goals.iter().any(|goal| goal.id == id) {
        state.goals.push(Goal {
            id: id.clone(),
            label: format!("Required dependency {project_id}"),
            kind: GoalKind::Dependency,
            status: GoalStatus::Covered,
        });
    }
    id
}

pub(super) fn mark_goal_status(state: &mut ModPlanState, goal_id: &str, status: GoalStatus) {
    if let Some(goal) = state.goals.iter_mut().find(|goal| goal.id == goal_id) {
        goal.status = status;
    }
}

pub(super) fn resolved_mod_from_payload(
    payload: serde_json::Value,
    provenance: ModProvenance,
    goal_id: Option<String>,
) -> Option<ResolvedMod> {
    let provider = optional_json_string(&payload, "provider")?;
    let project_id = optional_json_string(&payload, "project_id")?;
    let source_ref = source_ref_payload(&payload);
    let version_id = source_ref
        .as_ref()
        .and_then(|source| optional_json_string(source, "version_id"))
        .or_else(|| {
            payload
                .get("resolved_version")
                .and_then(|v| optional_json_string(v, "version_id"))
        });
    let filename = source_ref
        .as_ref()
        .and_then(|source| source.get("file"))
        .and_then(version_file_from_payload)
        .map(|file| file.filename);
    Some(ResolvedMod {
        provider,
        project_id,
        slug: optional_json_string(&payload, "slug"),
        title: optional_json_string(&payload, "title"),
        version_id,
        filename,
        source_ref,
        payload,
        provenance,
        goal_id,
    })
}

pub(in crate::agent::workflow) fn active_addition_payloads(
    state: &ModPlanState,
) -> Vec<serde_json::Value> {
    let removed = state.removals.iter().cloned().collect::<HashSet<_>>();
    state
        .additions
        .iter()
        .filter(|m| !removed.contains(&m.project_id))
        .map(|m| m.payload.clone())
        .collect()
}

pub(super) async fn search_customization_mods(
    queries: &[String],
    target: &TargetCompatibility,
) -> Result<Vec<ModCandidate>> {
    const MAX_RESULTS: usize = 8;
    const MAX_PER_QUERY: usize = 2;
    const MAX_PER_QUERY_LIMIT: u32 = 2;

    let registry = ProviderRegistry::with_defaults();
    let search_queries = dedupe_queries(
        queries
            .iter()
            .filter_map(|text| normalize_mod_search_query(text))
            .collect(),
    );

    // Freeze the provider iteration order once (HashMap order is stable for an unmutated registry,
    // and the old loop read `registry.all()` fresh each query in the same order). One SearchQuery
    // per text — its content is identical across providers, exactly like the old per-provider build.
    let providers: Vec<_> = registry.all().cloned().collect();
    let built: Vec<SearchQuery> = search_queries
        .iter()
        .map(|text| {
            let mut query = SearchQuery::new(text.clone(), ResourceKind::Mod);
            query.game_version = target.minecraft_version.clone();
            query.loader = target.loader.clone();
            query.limit = MAX_PER_QUERY_LIMIT;
            query
        })
        .collect();

    // Run every (query × provider) search concurrently (bounded), in query-major / provider order.
    // `buffered` preserves that order, so the dedup + per-query/total caps replayed below are
    // byte-for-byte identical. `collect` (not `try_collect`) keeps each Result so an error from a
    // search the caps would have skipped is discarded, just like the sequential early returns.
    let mut futs = Vec::new();
    for query in &built {
        for provider in &providers {
            futs.push(async move { (provider.id(), provider.search(query).await) });
        }
    }
    let flat: Vec<(ProviderId, Result<Vec<SearchHit>>)> = futures::stream::iter(futs)
        .buffered(PROVIDER_FANOUT)
        .collect()
        .await;

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut flat_iter = flat.into_iter();
    for text in &search_queries {
        let mut query_results = 0;
        for _ in 0..providers.len() {
            let (provider_id, hits_result) = flat_iter
                .next()
                .expect("flat holds exactly search_queries.len() * providers.len() entries");
            for hit in hits_result? {
                let key = format!("{provider_id:?}:{}", hit.id);
                if !seen.insert(key) {
                    continue;
                }
                out.push(ModCandidate {
                    provider: provider_id,
                    hit,
                    matched_query: text.clone(),
                });
                query_results += 1;
                if out.len() >= MAX_RESULTS {
                    return Ok(out);
                }
                if query_results >= MAX_PER_QUERY {
                    break;
                }
            }
        }
    }

    Ok(out)
}

pub(in crate::agent::workflow) fn fallback_mod_search_queries(
    queries: &[GoalQuery],
) -> Vec<String> {
    dedupe_queries(
        queries
            .iter()
            .filter_map(|query| normalize_mod_search_query(&query.query))
            .collect(),
    )
    .into_iter()
    .take(6)
    .collect()
}

pub(in crate::agent::workflow) fn unresolved_mod_plan_goals(
    state: &ModPlanState,
    diagnosis: Option<String>,
) -> Vec<serde_json::Value> {
    let fallback = "No compatible provider candidates were selected for this request. Revise the request, keep the current plan, or change the target and replan.";
    state
        .goals
        .iter()
        .filter(|goal| goal.kind == GoalKind::Theme && goal.status == GoalStatus::Open)
        .map(|goal| {
            serde_json::json!({
                "goal_id": goal.id,
                "label": goal.label,
                "status": goal.status,
                "diagnosis": diagnosis
                    .as_deref()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or(fallback),
                "next_step": "Revise the request, keep the current plan, or change the target and replan.",
            })
        })
        .collect()
}

pub(super) fn mod_ref_from_candidate(candidate: &ModCandidate) -> ModRef {
    ModRef::new(candidate.provider, candidate.hit.id.clone())
}

pub(super) fn dependency_resolution_payload(
    resolution: &crate::modplatform::dependency::DepResolution,
) -> serde_json::Value {
    serde_json::json!({
        "to_install": resolution.to_install.iter().map(resolved_file_ref_payload).collect::<Vec<_>>(),
        "satisfied": mod_ref_payloads(&resolution.satisfied),
        "unresolved": mod_ref_payloads(&resolution.unresolved),
        "incompatible": mod_ref_payloads(&resolution.incompatible),
    })
}

pub(in crate::agent::workflow) fn customization_blockers(
    resolution: &crate::modplatform::dependency::DepResolution,
) -> Vec<serde_json::Value> {
    let mut blockers = Vec::new();
    for unresolved in &resolution.unresolved {
        blockers.push(serde_json::json!({
            "kind": "unresolved_dependency",
            "provider": provider_slug(unresolved.provider),
            "project_id": unresolved.project_id.clone(),
            "reason": format!(
                "{}:{} has no compatible version or primary file",
                provider_slug(unresolved.provider),
                unresolved.project_id
            ),
        }));
    }
    for incompatible in &resolution.incompatible {
        blockers.push(serde_json::json!({
            "kind": "incompatible_dependency",
            "provider": provider_slug(incompatible.provider),
            "project_id": incompatible.project_id.clone(),
            "reason": format!(
                "{}:{} is declared incompatible by a selected mod",
                provider_slug(incompatible.provider),
                incompatible.project_id
            ),
        }));
    }
    blockers
}

pub(super) fn resolved_file_ref_payload(resolved: &ResolvedFile) -> serde_json::Value {
    serde_json::json!({
        "provider": provider_slug(resolved.provider),
        "project_id": resolved.project_id.clone(),
        "version_id": resolved.version_id.clone(),
        "filename": resolved.file.filename.clone(),
    })
}

pub(super) fn resolved_file_mod_payload(
    resolved: &ResolvedFile,
    root_keys: &HashSet<String>,
    root_hits: &HashMap<String, &ModCandidate>,
) -> serde_json::Value {
    let key = ModRef::new(resolved.provider, resolved.project_id.clone()).key();
    let root_hit = root_hits.get(&key).copied();
    let title = root_hit
        .map(|candidate| candidate.hit.title.clone())
        .or_else(|| resolved.project_name.clone())
        .unwrap_or_else(|| resolved.project_id.clone());
    let slug = root_hit
        .map(|candidate| candidate.hit.slug.clone())
        .or_else(|| resolved.project_slug.clone())
        .unwrap_or_else(|| resolved.project_id.clone());
    let matched_query = root_hit
        .map(|candidate| candidate.matched_query.clone())
        .unwrap_or_else(|| "dependency resolution".to_string());
    let auto_added = !root_keys.contains(&key);
    let provider = provider_slug(resolved.provider);
    let review_source = if auto_added {
        "dependency_resolution"
    } else {
        "selected_candidate"
    };
    let review_reason = if auto_added {
        "auto-added required dependency".to_string()
    } else {
        format!("matched {matched_query}")
    };
    let file = root_hit
        .map(|candidate| version_file_with_project_side(&resolved.file, &candidate.hit))
        .unwrap_or_else(|| resolved.file.clone());

    serde_json::json!({
        "provider": provider,
        "project_id": resolved.project_id.clone(),
        "slug": slug,
        "title": title,
        "description": if auto_added {
            "Automatically added required dependency"
        } else {
            "Selected extra mod"
        },
        "describe": format!(
            "{} | {} | {}",
            provider_label(resolved.provider),
            if auto_added { "auto-added dependency" } else { "selected candidate" },
            file.filename
        ),
        "author": null,
        "downloads": 0,
        "icon_url": null,
        "gallery_url": null,
        "categories": [],
        "url": project_url(resolved.provider, ResourceKind::Mod, &resolved.project_id),
        "matched_query": matched_query,
        "auto_added": auto_added,
        "dependency_reason": if auto_added { "required_dependency" } else { "root_candidate" },
        "review_source": review_source,
        "review_reason": review_reason,
        "review_version": resolved.version_id.clone(),
        "review_file": file.filename.clone(),
        "resolved_version": {
            "version_id": resolved.version_id.clone(),
            "primary_file": version_file_payload(&file),
        },
        "source_ref": {
            "kind": "mod_file",
            "provider": provider,
            "project_id": resolved.project_id.clone(),
            "version_id": resolved.version_id.clone(),
            "file": version_file_payload(&file),
        },
    })
}
