use super::*;

#[test]
fn base_search_strategy_is_bounded_and_adjusts_mode() {
    assert!(!base_search_has_acceptable_count(0));
    assert_eq!(next_base_search_mode(0), BaseSearchMode::Loose);
    assert!(base_search_has_acceptable_count(BASE_SEARCH_MIN_CANDIDATES));
    assert!(base_search_has_acceptable_count(BASE_SEARCH_MAX_CANDIDATES));
    assert!(!base_search_has_acceptable_count(
        BASE_SEARCH_MAX_CANDIDATES + 1
    ));
    assert_eq!(
        next_base_search_mode(BASE_SEARCH_MAX_CANDIDATES + 1),
        BaseSearchMode::Tight
    );
    assert_eq!(BASE_SEARCH_MAX_ITERATIONS, 4);
}

fn base_search_candidate(
    title: &str,
    downloads: u64,
    archive_size: Option<u64>,
) -> BasePackCandidate {
    let primary_file = archive_size.map(|size| VersionFile {
        url: format!("https://example.test/{title}.mrpack"),
        filename: format!("{title}.mrpack"),
        sha1: None,
        sha512: None,
        size: Some(size),
        primary: true,
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
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
            categories: vec!["fabric".to_string(), "adventure".to_string()],
            client_side: ProjectSideSupport::Unknown,
            server_side: ProjectSideSupport::Unknown,
        },
        matched_query: "Fabric 1.20.1 adventure".to_string(),
        resolved_target: Some(TargetCompatibility {
            minecraft_version: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
            version_id: Some(format!("{title}-version")),
            version_name: Some(format!("{title} Version")),
            version_number: Some("1.0.0".to_string()),
            game_versions: vec!["1.20.1".to_string()],
            loaders: vec!["fabric".to_string()],
            primary_file,
            dependencies: Vec::new(),
        }),
    }
}

#[test]
fn base_pack_ranking_demotes_oversized_archives() {
    let huge_popular = base_search_candidate(
        "Huge Popular Pack",
        100_000,
        Some(MAX_BASE_ARCHIVE_BYTES as u64 + 1),
    );
    let small_adventure = base_search_candidate("Small Adventure Pack", 250, Some(512 * 1024));

    let ranked = rank_base_packs(vec![huge_popular, small_adventure]);

    assert_eq!(ranked[0].hit.title, "Small Adventure Pack");
    assert_eq!(ranked[1].hit.title, "Huge Popular Pack");
}

#[test]
fn repeated_agent_reentry_keeps_snapshot_boundaries_consistent() {
    let mut run = AgentRunSnapshot::new("make a long-running adventure pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.restrictions = Some(BuildRestrictions {
        revision: 1,
        minecraft_version: Some("1.20.1".to_string()),
        minecraft_version_requirement: Some("1.20.1".to_string()),
        loader: Some("fabric".to_string()),
        feature_tags: vec!["adventure".to_string()],
        notes: None,
        history: Vec::new(),
    });
    run.approved_build = Some(ApprovedModpackBuild {
        base_pack: test_base_pack_payload(0),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: Vec::new(),
        execution_recipe: Some(serde_json::json!({
            "kind": "mrpack_from_base_modpack",
            "format": "mrpack"
        })),
    });
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::Ready,
        manifest: Some(serde_json::json!({ "status": "ready" })),
        blocked: None,
    });

    for cycle in 1..=3 {
        let current = run.restrictions.clone().unwrap_or_default();
        let target = if cycle % 2 == 0 {
            ("1.20.1", "fabric")
        } else {
            ("1.19.2", "forge")
        };
        let output = update_build_restrictions(
            Some(current.clone()),
            UpdateBuildRestrictionsInput {
                base_revision: current.revision,
                patch: BuildRestrictionPatch {
                    minecraft_version: Some(target.0.to_string()),
                    minecraft_version_requirement: Some(target.0.to_string()),
                    loader: Some(target.1.to_string()),
                    feature_tags: vec!["adventure".to_string(), format!("cycle-{cycle}")],
                    notes: Some(format!("replan cycle {cycle}")),
                },
            },
            BuildRestrictionChangeSource::UserRevise,
            format!("cycle {cycle}: target changed"),
        )
        .expect("restriction update should apply");
        run = apply_requirements_replan(
            run,
            output,
            format!("cycle {cycle}: target changed"),
            AgentPhase::ConfirmCustomizationApproval,
        );
        assert_eq!(run.status, AgentStatus::Running);
        assert_eq!(run.phase, AgentPhase::BasePackSearch);
        assert!(run.pending_approval.is_none());
        assert!(run.approved_build.is_none());
        assert!(run.execution.is_none());
        assert_eq!(run.replans.len(), cycle);

        run = block_base_pack_planning(
            run,
            test_base_pack_payload(cycle),
            format!("cycle {cycle}: archive unavailable"),
        );
        assert_eq!(run.status, AgentStatus::Running);
        assert!(run.pending_approval.is_none());
        let approval = approval_draft(&run, "choose_base_pack_approval");
        assert_eq!(approval.kind, ApprovalKind::ChooseBasePack);
        assert!(
            approval
                .options
                .iter()
                .all(|option| option.id != "scratch:fallback")
        );

        run.phase = AgentPhase::CustomizationPlanning;
        run.restrictions = Some(BuildRestrictions {
            loader: Some("fabric".to_string()),
            ..BuildRestrictions::default()
        });
        run = block_customization_planning(
            run,
            CustomizationPlanningBlocked {
                reason: format!("cycle {cycle}: missing concrete Minecraft version"),
                replan_phase: AgentPhase::ConfigureRequirementsApproval,
                details: serde_json::json!({ "cycle": cycle }),
            },
        );
        assert_eq!(run.status, AgentStatus::Running);
        assert!(run.pending_approval.is_none());
        let output = requirements_output_draft(&run);
        let output_value =
            serde_json::to_value(&output).expect("requirements output should serialize");
        let missing_fields = output_value
            .get("missing_fields")
            .and_then(|value| value.as_array())
            .expect("requirements block should expose missing fields");
        assert_eq!(missing_fields, &[serde_json::json!("minecraft_version")]);

        run.phase = AgentPhase::CustomizationPlanning;
        run = block_customization_planning(
            run,
            CustomizationPlanningBlocked {
                reason: format!("cycle {cycle}: incompatible dependency"),
                replan_phase: AgentPhase::ConfirmCustomizationApproval,
                details: serde_json::json!({ "cycle": cycle }),
            },
        );
        assert_eq!(run.status, AgentStatus::Running);
        assert!(run.pending_approval.is_none());
        let approval = approval_draft(&run, "confirm_customization_approval");
        assert_eq!(approval.kind, ApprovalKind::ConfirmCustomization);
        assert!(
            approval
                .options
                .iter()
                .all(|option| option.id != "confirm:recommended_customization")
        );

        run =
            continue_after_customization_confirmation(run, recommended_customization_option(cycle))
                .expect("customization confirmation should enter execution ready");
        assert_eq!(run.status, AgentStatus::Running);
        assert_eq!(run.phase, AgentPhase::ExecutionReady);
        assert_eq!(
            run.execution.as_ref().map(|execution| &execution.status),
            Some(&AgentExecutionStatus::NotStarted)
        );
        assert!(run.pending_approval.is_none());

        run = continue_after_execution_manifest_result(
            run,
            serde_json::json!({
                "status": "failed",
                "retryable": true,
                "reason": format!("cycle {cycle}: source timeout")
            }),
        )
        .expect("retryable execution error should stay recoverable");
        assert_eq!(run.status, AgentStatus::Running);
        assert_eq!(run.phase, AgentPhase::ExecutionReady);
        assert_eq!(
            run.execution.as_ref().map(|execution| &execution.status),
            Some(&AgentExecutionStatus::Retry)
        );

        run = continue_after_execution_manifest_result(
            run,
            serde_json::json!({
                "status": "blocked",
                "replan_phase": "confirm_customization_approval",
                "blocked": [{
                    "title": format!("Extra Mod {cycle}"),
                    "reason": "missing resolved source file"
                }]
            }),
        )
        .expect("execution block should return to customization gate");
        assert_waiting_at(
            &run,
            AgentPhase::ConfirmCustomizationApproval,
            ApprovalKind::ConfirmCustomization,
        );

        run =
            continue_after_customization_confirmation(run, recommended_customization_option(cycle))
                .expect("customization confirmation should re-enter execution ready");
        run = continue_after_execution_manifest_result(
            run,
            serde_json::json!({
                "status": "blocked",
                "replan_phase": "base_pack",
                "blocked": [{
                    "title": format!("Base Pack {cycle}"),
                    "reason": "base archive missing modrinth.index.json"
                }]
            }),
        )
        .expect("execution block should return to base-pack gate");
        assert_waiting_at(
            &run,
            AgentPhase::ChooseBasePackApproval,
            ApprovalKind::ChooseBasePack,
        );

        run =
            continue_after_customization_confirmation(run, recommended_customization_option(cycle))
                .expect("customization confirmation should re-enter execution ready again");
        run = continue_after_execution_manifest_result(
            run,
            serde_json::json!({
                "status": "blocked",
                "replan_phase": "requirements",
                "blocked": [{
                    "title": "target",
                    "reason": "selected loader is incompatible with requested version"
                }]
            }),
        )
        .expect("execution block should return to requirements gate");
        assert_waiting_at(
            &run,
            AgentPhase::ConfigureRequirementsApproval,
            ApprovalKind::ConfigureRequirements,
        );
    }

    run = continue_after_customization_confirmation(run, recommended_customization_option(99))
        .expect("final customization confirmation should enter execution ready");
    run = continue_after_execution_manifest_result(
        run,
        serde_json::json!({
            "status": "failed",
            "replan_phase": "base_pack",
            "failed": { "reason": "executor invariant violated" }
        }),
    )
    .expect("non-retryable failure should be classified");
    assert_eq!(run.status, AgentStatus::Failed);
    assert_eq!(run.phase, AgentPhase::Failed);
    assert_eq!(
        run.execution.as_ref().map(|execution| &execution.status),
        Some(&AgentExecutionStatus::Failed)
    );
    assert_eq!(
        run.execution
            .as_ref()
            .and_then(|execution| execution.blocked.as_ref())
            .and_then(|blocked| blocked.replan_phase.as_ref()),
        Some(&AgentPhase::ChooseBasePackApproval)
    );
}

#[test]
fn parse_base_modlist_supports_modrinth_and_cache_is_single_fetch() {
    let archive = zip_bytes(&[(
        "modrinth.index.json",
        br#"{
                "formatVersion": 1,
                "game": "minecraft",
                "versionId": "1.0.0",
                "name": "Base",
                "dependencies": { "minecraft": "1.20.1", "fabric-loader": "0.15.7" },
                "files": [{
                    "path": "mods/sodium.jar",
                    "hashes": { "sha512": "abc" },
                    "downloads": ["https://cdn.modrinth.com/data/sodium/versions/v/sodium.jar"],
                    "fileSize": 1
                }]
            }"#,
    )]);

    let refs = parse_base_modlist(&archive).expect("mrpack modlist should parse");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].provider, ProviderId::Modrinth);
    assert_eq!(refs[0].project_id, "sodium");

    let cache =
        base_modlist_cache_from_archive_bytes(&archive).expect("cache should be built once");
    assert_eq!(cache.fetch_count, 1);
    assert_eq!(cache.refs, refs);
    assert_eq!(
        cache.refs,
        cache.refs.clone(),
        "cached refs are reused immutably"
    );
}

#[test]
fn parse_base_modlist_supports_curseforge_manifest() {
    let archive = zip_bytes(&[(
        "manifest.json",
        br#"{
                "manifestType": "minecraftModpack",
                "manifestVersion": 1,
                "name": "CF Base",
                "version": "1.0.0",
                "author": "author",
                "minecraft": {
                    "version": "1.20.1",
                    "modLoaders": [{ "id": "forge-47.2.0", "primary": true }]
                },
                "files": [
                    { "projectID": 238222, "fileID": 4575706, "required": true },
                    { "projectID": 999999, "fileID": 1, "required": false }
                ],
                "overrides": "overrides"
            }"#,
    )]);

    let refs = parse_base_modlist(&archive).expect("curseforge manifest should parse");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].provider, ProviderId::CurseForge);
    assert_eq!(refs[0].project_id, "238222");
}

#[test]
fn base_modlist_rejects_oversized_manifest_entry() {
    let oversized = vec![b' '; MAX_BASE_MANIFEST_BYTES as usize + 1];
    let archive = zip_bytes(&[("manifest.json", &oversized)]);

    let err = parse_base_modlist(&archive).expect_err("oversized manifest must be rejected");

    assert!(err.to_string().contains("exceeds maximum size"));
}

#[test]
fn base_archive_size_cap_rejects_large_archive() {
    let err = ensure_base_archive_size(
        "https://example.com/base.mrpack",
        MAX_BASE_ARCHIVE_BYTES + 1,
    )
    .expect_err("oversized base archive must be rejected");

    assert!(err.to_string().contains("maximum size"));
}

#[test]
fn planning_entry_failure_prepares_base_pack_approval_draft() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::Running;
    run.phase = AgentPhase::CustomizationPlanning;
    let next = block_base_pack_planning(
        run,
        serde_json::json!({
            "provider": "modrinth",
            "project_id": "base",
            "title": "Base"
        }),
        "network timeout".to_string(),
    );

    assert_eq!(next.status, AgentStatus::Running);
    assert!(next.pending_approval.is_none());
    assert!(next.approved_build.is_none());
    assert!(next.execution.is_none());
    let approval = approval_draft(&next, "choose_base_pack_approval");
    assert_eq!(approval.kind, ApprovalKind::ChooseBasePack);
    assert!(approval.message.contains("network timeout"));
}

#[test]
fn empty_base_pack_search_offers_scratch_fallback() {
    let approval = base_pack_selection_approval(&[], test_plan());

    assert_eq!(approval.kind, ApprovalKind::ChooseBasePack);
    assert!(
        approval
            .options
            .iter()
            .any(|option| option.id == "scratch:fallback")
    );
    assert!(
        approval
            .available_decisions
            .iter()
            .any(|d| d.kind == UserDecisionKind::Approve)
    );
    assert!(approval.message.contains("Start from scratch"));
}

#[test]
fn scratch_fallback_choice_records_selection_for_agent_loop() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.pending_approval = Some(ApprovalRequest {
        id: "approval-test".to_string(),
        kind: ApprovalKind::ConfirmScratchFallback,
        title: "Confirm scratch build".to_string(),
        message: "legacy scratch fallback".to_string(),
        options: vec![ApprovalOption {
            id: "confirm:scratch_fallback".to_string(),
            label: "Confirm scratch build".to_string(),
            description: None,
            payload: Some(serde_json::json!({ "mode": "scratch_fallback" })),
        }],
        available_decisions: approval_decisions("Confirm scratch build", "Search base packs again"),
        tools: Vec::new(),
        plan: None,
    });

    let approval = run.pending_approval.clone().unwrap();
    let next = apply_modpack_build_user_decision(
        run,
        &approval,
        UserDecision {
            approval_id: "approval-test".to_string(),
            kind: UserDecisionKind::Approve,
            selected_option_id: Some("confirm:scratch_fallback".to_string()),
            message: None,
            edits: serde_json::json!({}),
        },
    )
    .expect("scratch fallback selection should be stored for the agent loop");

    assert_eq!(next.status, AgentStatus::Running);
    assert!(next.pending_approval.is_none());
    assert!(next.agent_memory.get("selected_base_pack_option").is_some());
}

#[test]
fn modplan_initial_state_seeds_baseline_goal_and_addition() {
    let (_, _, mut state) = initialized_mod_plan(&["ocean"]);
    let roots = baseline_mod_refs("fabric");
    let resolution = crate::modplatform::dependency::DepResolution {
        to_install: vec![ResolvedFile {
            provider: ProviderId::Modrinth,
            project_id: "P7dR8mSH".to_string(),
            version_id: "fabric-api-version".to_string(),
            file: test_version_file("P7dR8mSH"),
            project_name: Some("Fabric API".to_string()),
            project_slug: Some("fabric-api".to_string()),
            authors: Vec::new(),
        }],
        ..Default::default()
    };
    let goal_map =
        std::collections::HashMap::from([(roots[0].key(), "baseline:fabric".to_string())]);
    let root_hits: std::collections::HashMap<String, &ModCandidate> =
        std::collections::HashMap::new();

    append_dependency_resolution(
        &mut state,
        &resolution,
        &roots,
        &root_hits,
        &goal_map,
        ModProvenance::Baseline,
    );

    assert!(
        state
            .goals
            .iter()
            .any(|goal| goal.id == "baseline:fabric" && goal.status == GoalStatus::Covered)
    );
    assert!(
        state
            .additions
            .iter()
            .any(|m| m.project_id == "P7dR8mSH" && m.provenance == ModProvenance::Baseline)
    );
    assert!(
        state
            .pending_queries
            .iter()
            .any(|query| query.query == "ocean")
    );
}

#[test]
fn modplan_prefilter_removes_existing_and_blocked_candidates() {
    let target = test_mod_plan_target();
    let base_modlist = BaseModlistCache {
        refs: vec![ModRef::new(ProviderId::Modrinth, "already")],
        source_format: "mrpack".to_string(),
        fetch_count: 1,
    };
    let mut state = initialize_mod_plan_state(&target, &base_modlist, None);
    state.blocked.push("blocked".to_string());

    let filtered = prefilter_mod_candidates(
        vec![
            test_mod_candidate("already", "Already Installed"),
            test_mod_candidate("blocked", "Blocked Mod"),
            test_mod_candidate("new", "New Mod"),
        ],
        &state,
    );

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].hit.id, "new");
}

#[test]
fn modplan_step_prompt_includes_last_blockers() {
    let (target, _, state) = initialized_mod_plan(&["ocean"]);
    let blockers = vec![serde_json::json!({
        "kind": "incompatible_dependency",
        "project_id": "bad",
        "reason": "bad conflicts with the current mod set",
    })];

    let payload = super::customization::mod_plan_step_prompt_payload(
        "make a pack",
        &test_selected_base_pack(),
        &target,
        &state,
        &[test_mod_candidate("new", "New Mod")],
        &blockers,
    );

    assert_eq!(payload["last_blockers"], serde_json::json!(blockers));
}

#[tokio::test]
async fn modplan_apply_step_adds_required_dependency_not_optional() {
    let registry = test_provider_registry(vec![
        (
            "root",
            vec![
                test_dependency("required-dep", "required"),
                test_dependency("optional-dep", "optional"),
            ],
        ),
        ("required-dep", Vec::new()),
        ("optional-dep", Vec::new()),
    ]);
    let (_, _, mut state) = initialized_mod_plan(&["ocean"]);
    let goal_id = first_theme_goal_id(&state);
    let candidates = vec![test_mod_candidate("root", "Root Mod")];
    let step = test_mod_plan_step(
        vec![test_mod_selection(goal_id.clone(), "root")],
        "root covers ocean",
    );

    let applied = apply_mod_plan_step(&registry, &mut state, &candidates, step, "1.20.1", "fabric")
        .await
        .unwrap();

    assert!(applied.blockers.is_empty());
    let ids = state
        .additions
        .iter()
        .map(|m| m.project_id.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"root"));
    assert!(ids.contains(&"required-dep"));
    assert!(!ids.contains(&"optional-dep"));
    assert!(
        state
            .goals
            .iter()
            .any(|goal| goal.id == goal_id && goal.status == GoalStatus::Covered)
    );
    assert!(state.goals.iter().any(|goal| {
        goal.id == "dependency:required-dep" && goal.status == GoalStatus::Covered
    }));
}

#[tokio::test]
async fn modplan_apply_step_blocks_incompatible_selection() {
    let registry = test_provider_registry(vec![(
        "root",
        vec![test_dependency("conflict", "incompatible")],
    )]);
    let (_, _, mut state) = initialized_mod_plan(&["ocean"]);
    let goal_id = first_theme_goal_id(&state);
    let step = test_mod_plan_step(
        vec![test_mod_selection(goal_id.clone(), "root")],
        "try root",
    );

    let applied = apply_mod_plan_step(
        &registry,
        &mut state,
        &[test_mod_candidate("root", "Root Mod")],
        step,
        "1.20.1",
        "fabric",
    )
    .await
    .unwrap();

    assert!(!applied.blockers.is_empty());
    assert!(state.additions.is_empty());
    assert_eq!(state.blocked, vec!["root".to_string()]);
    let filtered = prefilter_mod_candidates(vec![test_mod_candidate("root", "Root Mod")], &state);
    assert!(filtered.is_empty());
    assert!(
        state
            .goals
            .iter()
            .any(|goal| goal.id == goal_id && goal.status == GoalStatus::Open)
    );
}

#[tokio::test]
async fn modplan_apply_step_keeps_compatible_selection_when_peer_conflicts() {
    let registry = test_provider_registry(vec![
        ("ok", Vec::new()),
        ("bad", vec![test_dependency("conflict", "incompatible")]),
    ]);
    let (_, _, mut state) = initialized_mod_plan(&["ocean", "danger"]);
    let goals = theme_goal_ids(&state);
    let step = test_mod_plan_step(
        vec![
            test_mod_selection(goals[0].clone(), "ok"),
            test_mod_selection(goals[1].clone(), "bad"),
        ],
        "try both",
    );

    let applied = apply_mod_plan_step(
        &registry,
        &mut state,
        &[
            test_mod_candidate("ok", "OK Mod"),
            test_mod_candidate("bad", "Bad Mod"),
        ],
        step,
        "1.20.1",
        "fabric",
    )
    .await
    .unwrap();

    assert!(!applied.blockers.is_empty());
    assert!(state.additions.iter().any(|m| m.project_id == "ok"));
    assert!(!state.additions.iter().any(|m| m.project_id == "bad"));
    assert_eq!(state.blocked, vec!["bad".to_string()]);
    assert!(
        state
            .goals
            .iter()
            .any(|goal| goal.id == goals[0] && goal.status == GoalStatus::Covered)
    );
    assert!(
        state
            .goals
            .iter()
            .any(|goal| goal.id == goals[1] && goal.status == GoalStatus::Open)
    );
}

#[tokio::test]
async fn modplan_selection_can_restore_previous_removal() {
    let registry = test_provider_registry(vec![("root", Vec::new())]);
    let (_, _, mut state) = initialized_mod_plan(&["ocean"]);
    state.removals.push("root".to_string());
    let goal_id = first_theme_goal_id(&state);
    let step = test_mod_plan_step(vec![test_mod_selection(goal_id, "root")], "restore root");

    let applied = apply_mod_plan_step(
        &registry,
        &mut state,
        &[test_mod_candidate("root", "Root Mod")],
        step,
        "1.20.1",
        "fabric",
    )
    .await
    .unwrap();

    assert!(applied.blockers.is_empty());
    assert!(!state.removals.iter().any(|id| id == "root"));
    assert!(state.additions.iter().any(|m| m.project_id == "root"));
}

#[tokio::test]
async fn modplan_required_dependency_overrides_removal() {
    let registry = test_provider_registry(vec![
        ("root", vec![test_dependency("required-dep", "required")]),
        ("required-dep", Vec::new()),
    ]);
    let (_, _, mut state) = initialized_mod_plan(&["ocean"]);
    state.removals.push("required-dep".to_string());
    let goal_id = first_theme_goal_id(&state);
    let step = test_mod_plan_step(
        vec![test_mod_selection(goal_id, "root")],
        "root needs dependency",
    );

    apply_mod_plan_step(
        &registry,
        &mut state,
        &[test_mod_candidate("root", "Root Mod")],
        step,
        "1.20.1",
        "fabric",
    )
    .await
    .unwrap();

    let payload_ids = state
        .additions
        .iter()
        .map(|entry| entry.project_id.as_str())
        .collect::<Vec<_>>();
    assert!(payload_ids.contains(&"root"));
    assert!(payload_ids.contains(&"required-dep"));
    assert!(!state.removals.iter().any(|id| id == "required-dep"));
}

#[test]
fn invalidate_target_change_clears_mod_plan() {
    let (_, _, state) = initialized_mod_plan_without_restrictions();
    let mut run = AgentRunSnapshot::new("make a pack");
    run.mod_plan = Some(state);

    super::requirements::invalidate_downstream(
        &mut run,
        ChangedField::Loader,
        "loader changed",
        AgentPhase::ConfirmCustomizationApproval,
        None,
    );

    assert!(run.mod_plan.is_none());
}
