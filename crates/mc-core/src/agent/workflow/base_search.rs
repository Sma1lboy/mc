use super::*;

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
        let filtered = filter_base_packs_by_restrictions(searched, requested).await?;
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
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for text in queries {
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
        for hit in provider.search(&query).await? {
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
) -> Result<Vec<BasePackCandidate>> {
    let candidates = candidates
        .into_iter()
        .filter(|candidate| base_pack_provider_supported_for_execution(candidate.provider))
        .collect::<Vec<_>>();
    if requested.minecraft_version.is_none() && requested.loader.is_none() {
        return Ok(candidates);
    }

    let registry = ProviderRegistry::with_defaults();
    let mut out = Vec::new();
    for candidate in candidates {
        let Some(provider) = registry.get(candidate.provider) else {
            continue;
        };
        let versions = provider
            .list_versions(
                &candidate.hit.id,
                requested.minecraft_version.as_deref(),
                requested.loader.as_deref(),
            )
            .await?;
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

pub(super) fn rank_base_packs(mut candidates: Vec<BasePackCandidate>) -> Vec<BasePackCandidate> {
    candidates.sort_by(|a, b| {
        base_archive_rank_bucket(a)
            .cmp(&base_archive_rank_bucket(b))
            .then_with(|| {
                base_archive_size(a)
                    .unwrap_or(u64::MAX)
                    .cmp(&base_archive_size(b).unwrap_or(u64::MAX))
            })
            .then_with(|| b.hit.downloads.cmp(&a.hit.downloads))
            .then_with(|| a.hit.title.cmp(&b.hit.title))
    });
    candidates
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

fn base_archive_rank_bucket(candidate: &BasePackCandidate) -> u8 {
    match base_archive_size(candidate) {
        Some(size) if size <= MAX_BASE_ARCHIVE_BYTES as u64 => 0,
        None => 1,
        Some(_) => 2,
    }
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
