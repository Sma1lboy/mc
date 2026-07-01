use super::*;

#[tokio::test]
async fn modplan_round_cap_with_open_theme_returns_blocked() {
    let (target, base_modlist, mut state) = initialized_mod_plan(&["ocean"]);
    let mut run = AgentRunSnapshot::new("make a pack");
    state.round = MOD_PLAN_ROUND_CAP;
    run.mod_plan = Some(state);
    let runtime = test_main_runtime();

    let result = run_customization_planning_loop(
        &runtime.llm,
        &mut run,
        "make a pack",
        &test_selected_base_pack(),
        &target,
        &[],
        &base_modlist,
    )
    .await
    .unwrap();

    let CustomizationPlanningResult::Blocked(blocked) = result else {
        panic!("round-cap open goal should block customization planning");
    };
    assert!(blocked.reason.contains("round cap"));
    assert!(
        !blocked.details["unresolved_goals"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn modplan_revise_merges_feedback_without_resetting_round() {
    let (_, _, state) = initialized_mod_plan_without_restrictions();
    let mut run = AgentRunSnapshot::new("make a pack");
    run.mod_plan = Some(state);
    run.mod_plan.as_mut().unwrap().round = 2;

    merge_feedback_into_mod_plan(&mut run, "more underwater survival");
    merge_feedback_into_mod_plan(
        &mut run,
        "replace most Create addons with ocean exploration mods",
    );

    let state = run.mod_plan.unwrap();
    assert_eq!(state.round, 2);
    assert!(state
        .goals
        .iter()
        .any(|goal| goal.label == "more underwater survival" && goal.status == GoalStatus::Open));
    assert!(state.goals.iter().any(|goal| {
        goal.label == "replace most Create addons with ocean exploration mods"
            && goal.status == GoalStatus::Open
    }));
    assert!(
        state
            .pending_queries
            .iter()
            .any(|query| query.query == "more underwater survival")
    );
    assert!(
        state.pending_queries.iter().any(|query| {
            query.query == "replace most Create addons with ocean exploration mods"
        })
    );
}

#[test]
fn old_snapshot_without_mod_plan_deserializes() {
    let snapshot = serde_json::json!({
        "schema_version": 1,
        "id": "agent-run-old",
        "workflow": "modpack_build",
        "status": "running",
        "phase": "base_pack_search",
        "user_prompt": "make a pack"
    });

    let run: AgentRunSnapshot = serde_json::from_value(snapshot).unwrap();

    assert!(run.mod_plan.is_none());
    assert_eq!(run.id, "agent-run-old");
}

#[test]
fn customization_conflict_is_blocked_without_confirm_option() {
    let resolution = crate::modplatform::dependency::DepResolution {
        incompatible: vec![ModRef::new(ProviderId::Modrinth, "badmod")],
        ..Default::default()
    };
    let blockers = customization_blockers(&resolution);
    assert_eq!(blockers.len(), 1);
    assert_eq!(
        blockers[0].get("kind").and_then(|v| v.as_str()),
        Some("incompatible_dependency")
    );

    let mut run = AgentRunSnapshot::new("make a pack");
    run.phase = AgentPhase::CustomizationPlanning;
    let next = block_customization_planning(
        run,
        CustomizationPlanningBlocked {
            reason: "incompatible dependency".to_string(),
            replan_phase: AgentPhase::ConfirmCustomizationApproval,
            details: serde_json::json!({ "blockers": blockers }),
        },
    );
    assert_eq!(next.status, AgentStatus::Running);
    assert!(next.pending_approval.is_none());
    let approval = approval_draft(&next, "confirm_customization_approval");
    assert_eq!(approval.kind, ApprovalKind::ConfirmCustomization);
    assert!(
        approval
            .options
            .iter()
            .all(|o| o.id != "confirm:recommended_customization")
    );
}

#[test]
fn customization_block_to_requirements_preserves_missing_fields_in_draft() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.phase = AgentPhase::CustomizationPlanning;
    run.restrictions = Some(BuildRestrictions {
        loader: Some("fabric".to_string()),
        ..BuildRestrictions::default()
    });

    let next = block_customization_planning(
        run,
        CustomizationPlanningBlocked {
            reason: "missing concrete Minecraft version".to_string(),
            replan_phase: AgentPhase::ConfigureRequirementsApproval,
            details: serde_json::json!({}),
        },
    );

    assert_eq!(next.status, AgentStatus::Running);
    assert!(next.pending_approval.is_none());
    let output = requirements_output_draft(&next);
    let output_value = serde_json::to_value(&output).expect("requirements output should serialize");
    let missing = output_value
        .get("missing_fields")
        .and_then(|v| v.as_array())
        .expect("requirements output should carry missing fields");
    assert_eq!(missing, &[serde_json::json!("minecraft_version")]);
}

#[test]
fn invalidate_downstream_is_idempotent() {
    let mut run = approved_run_with_execution();

    for _ in 0..2 {
        invalidate_downstream(
            &mut run,
            ChangedField::MinecraftVersion,
            "target changed",
            AgentPhase::ConfirmCustomizationApproval,
            None,
        );
    }

    assert!(run.approved_build.is_none());
    assert!(run.execution.is_none());
    assert_eq!(run.replans.len(), 1);
    assert_eq!(
        run.replans[0].invalidates,
        vec![
            PlanArtifact::BasePack,
            PlanArtifact::ExtraMods,
            PlanArtifact::ApprovedBuild,
            PlanArtifact::ExecutionMetadata,
        ]
    );
}

#[test]
fn invalidate_downstream_dedupes_by_semantic_identity_not_reason_text() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    let patch = Some(BuildRestrictionPatch {
        minecraft_version: Some("1.20.1".to_string()),
        minecraft_version_requirement: Some("1.20.1".to_string()),
        loader: Some("fabric".to_string()),
        feature_tags: vec!["tech".to_string()],
        notes: None,
    });

    invalidate_downstream(
        &mut run,
        ChangedField::MinecraftVersion,
        "first model wording",
        AgentPhase::ConfirmCustomizationApproval,
        patch.clone(),
    );
    invalidate_downstream(
        &mut run,
        ChangedField::MinecraftVersion,
        "second model wording",
        AgentPhase::ConfirmCustomizationApproval,
        patch,
    );

    assert_eq!(run.replans.len(), 1);
    assert_eq!(run.replans[0].reason, "first model wording");
}

#[test]
fn content_preference_invalidation_preserves_selected_base_pack() {
    let mut run = approved_run_with_execution();

    invalidate_downstream(
        &mut run,
        ChangedField::ContentPreference,
        "content preference changed",
        AgentPhase::ConfirmCustomizationApproval,
        None,
    );

    assert!(run.approved_build.is_none());
    assert!(run.execution.is_none());
    assert_eq!(
        run.replans[0].invalidates,
        vec![
            PlanArtifact::ExtraMods,
            PlanArtifact::ApprovedBuild,
            PlanArtifact::ExecutionMetadata,
        ]
    );
    assert_eq!(
        run.replans[0].target_phase,
        AgentPhase::ConfirmCustomizationApproval
    );
}

#[test]
fn invalidation_dependency_graph_covers_all_changed_fields() {
    let expected = [
        (
            ChangedField::MinecraftVersion,
            AgentPhase::ConfigureRequirementsApproval,
            vec![
                PlanArtifact::BasePack,
                PlanArtifact::ExtraMods,
                PlanArtifact::ApprovedBuild,
                PlanArtifact::ExecutionMetadata,
            ],
        ),
        (
            ChangedField::Loader,
            AgentPhase::ConfigureRequirementsApproval,
            vec![
                PlanArtifact::BasePack,
                PlanArtifact::ExtraMods,
                PlanArtifact::ApprovedBuild,
                PlanArtifact::ExecutionMetadata,
            ],
        ),
        (
            ChangedField::VersionRequirement,
            AgentPhase::ConfigureRequirementsApproval,
            vec![
                PlanArtifact::BasePack,
                PlanArtifact::ExtraMods,
                PlanArtifact::ApprovedBuild,
                PlanArtifact::ExecutionMetadata,
            ],
        ),
        (
            ChangedField::ContentPreference,
            AgentPhase::ConfirmCustomizationApproval,
            vec![
                PlanArtifact::ExtraMods,
                PlanArtifact::ApprovedBuild,
                PlanArtifact::ExecutionMetadata,
            ],
        ),
        (
            ChangedField::SearchPreference,
            AgentPhase::ChooseBasePackApproval,
            vec![
                PlanArtifact::BasePack,
                PlanArtifact::ExtraMods,
                PlanArtifact::ApprovedBuild,
                PlanArtifact::ExecutionMetadata,
            ],
        ),
        (
            ChangedField::BasePack,
            AgentPhase::ChooseBasePackApproval,
            vec![
                PlanArtifact::ExtraMods,
                PlanArtifact::ApprovedBuild,
                PlanArtifact::ExecutionMetadata,
            ],
        ),
    ];

    assert_eq!(ALL_CHANGED_FIELDS.len(), expected.len());
    for (field, target_phase, invalidates) in expected {
        let rule = invalidation_rule_for_changed_field(field);
        assert_eq!(rule.invalidates.to_vec(), invalidates);
        assert_eq!(
            target_phase_for_changed_field(field, &AgentPhase::ConfirmCustomizationApproval),
            target_phase
        );
    }
}

#[test]
fn content_preference_invalidation_graph_can_refresh_in_place() {
    assert_eq!(
        target_phase_for_changed_field(
            ChangedField::ContentPreference,
            &AgentPhase::ConfigureRequirementsApproval,
        ),
        AgentPhase::ConfigureRequirementsApproval
    );
    assert_eq!(
        target_phase_for_changed_field(
            ChangedField::ContentPreference,
            &AgentPhase::BasePackSearch,
        ),
        AgentPhase::ChooseBasePackApproval
    );
    assert_eq!(
        target_phase_for_changed_field(
            ChangedField::ContentPreference,
            &AgentPhase::ChooseBasePackApproval,
        ),
        AgentPhase::ChooseBasePackApproval
    );
}

#[test]
fn execution_failed_manifest_routes_retry_or_terminal_failed() {
    let cases = [
        (
            serde_json::json!({
                "status": "failed",
                "retryable": true,
                "error_kind": "source_timeout",
                "reason": "source timed out"
            }),
            AgentStatus::Running,
            AgentPhase::Executing,
            AgentExecutionStatus::Retry,
            None,
        ),
        (
            serde_json::json!({
                "status": "failed",
                "replan_phase": "base_pack",
                "reason": "corrupt archive"
            }),
            AgentStatus::Failed,
            AgentPhase::Failed,
            AgentExecutionStatus::Failed,
            Some(Some(AgentPhase::ChooseBasePackApproval)),
        ),
    ];

    for (manifest, status, phase, execution_status, expected_replan_phase) in cases {
        let next = continue_after_execution_manifest_result(
            execution_manifest_run(AgentPhase::Executing),
            manifest,
        )
        .expect("failed manifest should be classified");

        assert_eq!(next.status, status);
        assert_eq!(next.phase, phase);
        assert_eq!(
            next.execution.as_ref().map(|e| &e.status),
            Some(&execution_status)
        );
        if let Some(expected_replan_phase) = expected_replan_phase {
            assert_eq!(
                next.execution
                    .as_ref()
                    .and_then(|e| e.blocked.as_ref())
                    .and_then(|b| b.replan_phase.as_ref()),
                expected_replan_phase.as_ref()
            );
        }
    }
}

#[test]
fn parses_intent_and_approval_router_json() {
    let intent = parse_intent_response(
        r#"{"intent":"build_modpack","confidence":0.91,"rationale":"user wants a pack"}"#,
    )
    .expect("build-modpack intent json should parse");
    assert_eq!(intent.kind, AgentIntentKind::BuildModpack);
    assert!((intent.confidence - 0.91).abs() < 0.001);
    assert_eq!(intent.rationale.as_deref(), Some("user wants a pack"));

    let unsupported = parse_intent_response(r#"{"intent":"general_question","confidence":0.8}"#)
        .expect("unsupported intent json should parse");
    assert_eq!(unsupported.kind, AgentIntentKind::Unknown);
    assert!((unsupported.confidence - 0.8).abs() < 0.001);

    let wiki = parse_intent_response(r#"{"intent":"wiki_query","confidence":0.7}"#)
        .expect("future wiki intent json should parse as unsupported");
    assert_eq!(wiki.kind, AgentIntentKind::Unknown);

    let upgrade = parse_intent_response(r#"{"intent":"upgrade_current_pack","confidence":0.7}"#)
        .expect("future upgrade intent json should parse as unsupported");
    assert_eq!(upgrade.kind, AgentIntentKind::Unknown);

    let approval = test_approval();
    let approve = parse_approval_decision_response(
            r#"{"decision":"approve","selected_option_id":"modrinth:second","message":null,"rationale":"user chose the second option"}"#,
            &approval,
        )
        .expect("approval decision should parse");
    assert_eq!(approve.approval_id, "approval-test");
    assert_eq!(approve.kind, UserDecisionKind::Approve);
    assert_eq!(
        approve.selected_option_id.as_deref(),
        Some("modrinth:second")
    );

    let revise = parse_approval_decision_response(
            r#"{"decision":"revise","selected_option_id":null,"message":"Search again with more adventure and exploration","rationale":"user asked to search again"}"#,
            &approval,
        )
        .expect("revise decision should parse");
    assert_eq!(revise.kind, UserDecisionKind::Revise);
    assert_eq!(
        revise.message.as_deref(),
        Some("Search again with more adventure and exploration")
    );

    let revise_with_option = parse_approval_decision_response(
            r#"{"decision":"revise","selected_option_id":"modrinth:first","message":"Search explicitly for Fabulously Optimized","rationale":"user asked to search again"}"#,
            &approval,
        )
        .expect("revise decision should ignore selected_option_id emitted by the router");
    assert_eq!(revise_with_option.kind, UserDecisionKind::Revise);
    assert_eq!(revise_with_option.selected_option_id, None);
    assert_eq!(
        revise_with_option.message.as_deref(),
        Some("Search explicitly for Fabulously Optimized")
    );
}

#[test]
fn home_launch_context_injects_build_workflow_only() {
    let home = AgentLaunchContext::from_entry(AgentEntry::Home);
    assert_eq!(home.entry, AgentEntry::Home);
    assert_eq!(
        home.available_workflows,
        vec![AgentWorkflowId::BuildModpack]
    );
    assert!(home.allows_workflow(AgentWorkflowId::BuildModpack));
}

#[test]
fn home_intent_routing_prompt_lists_only_build_workflow() {
    let home_prompt = intent_routing_prompt(&AgentLaunchContext::from_entry(AgentEntry::Home));
    assert!(home_prompt.contains("- build_modpack:"));
    assert!(!home_prompt.contains("- wiki_query:"));
    assert!(!home_prompt.contains("- upgrade_current_pack:"));
}

#[test]
fn scratch_prompt_contract_matches_supported_fallback() {
    assert!(!MAIN_AGENT_SYSTEM_PROMPT.contains("Build-from-scratch is not available"));
    assert!(!SEARCH_QUERY_PROMPT.contains("Build-from-scratch is not available"));
    assert!(MAIN_AGENT_SYSTEM_PROMPT.contains("scratch"));
    assert!(SEARCH_QUERY_PROMPT.contains("scratch"));
}

#[test]
fn approval_route_parser_rejects_unknown_option() {
    let approval = test_approval();
    let text_err = parse_approval_decision_response(
            r#"{"decision":"approve","selected_option_id":"confirm:recommended_customization","message":null,"rationale":"wrong gate"}"#,
            &approval,
        )
        .expect_err("text approval route should reject options outside the pending gate");
    assert!(text_err.to_string().contains("unknown option"));
}

#[test]
fn approval_decision_needs_clarification_does_not_advance() {
    let approval = test_approval();
    let err = parse_approval_decision_response(
            r#"{"decision":"needs_clarification","selected_option_id":null,"message":null,"rationale":"message does not choose or revise"}"#,
            &approval,
        )
        .expect_err("needs_clarification should not produce a workflow decision");

    assert!(err.to_string().contains("needs clarification"));
}

#[test]
fn search_query_parser_cleans_dedupes_and_truncates() {
    let queries = search_queries(
        r#"{"queries":["1. Alpha","alpha","- Beta","queries","Gamma","Delta","Epsilon"]}"#,
    )
    .expect("search query JSON should normalize");

    assert_eq!(queries, vec!["Alpha", "Beta", "Gamma"]);
}

#[test]
fn restriction_update_input_is_normalized_without_reparsing() {
    let input = normalize_restriction_update_input(UpdateBuildRestrictionsInput {
        base_revision: 9,
        patch: BuildRestrictionPatch {
            minecraft_version: Some("99.99".to_string()),
            minecraft_version_requirement: Some("1.20.x".to_string()),
            loader: Some("Fabric".to_string()),
            feature_tags: vec![
                " exploration ".to_string(),
                "exploration".to_string(),
                "qol".to_string(),
            ],
            notes: Some("  keep this note  ".to_string()),
        },
    });

    assert_eq!(input.base_revision, 9);
    assert_eq!(input.patch.minecraft_version, None);
    assert_eq!(
        input.patch.minecraft_version_requirement.as_deref(),
        Some("1.20.x")
    );
    assert_eq!(input.patch.loader.as_deref(), Some("fabric"));
    assert_eq!(input.patch.feature_tags, vec!["exploration", "qol"]);
    assert_eq!(input.patch.notes.as_deref(), Some("keep this note"));
}

#[test]
fn restriction_retry_accepts_valid_retry_without_reparsing() {
    let first_err = CoreError::other("first typed restriction output failed validation");
    let input = validate_restriction_update_retry(
        UpdateBuildRestrictionsInput {
            base_revision: 3,
            patch: BuildRestrictionPatch {
                minecraft_version: Some(" 1.20.1 ".to_string()),
                minecraft_version_requirement: None,
                loader: Some("Fabric".to_string()),
                feature_tags: vec![" performance ".to_string(), "performance".to_string()],
                notes: Some("  retry normalized this  ".to_string()),
            },
        },
        &first_err,
    )
    .expect("retry value should validate without reparsing JSON text");

    assert_eq!(input.base_revision, 3);
    assert_eq!(input.patch.minecraft_version.as_deref(), Some("1.20.1"));
    assert_eq!(
        input.patch.minecraft_version_requirement.as_deref(),
        Some("1.20.1")
    );
    assert_eq!(input.patch.loader.as_deref(), Some("fabric"));
    assert_eq!(input.patch.feature_tags, vec!["performance"]);
    assert_eq!(input.patch.notes.as_deref(), Some("retry normalized this"));
}

#[tokio::test]
async fn unrelated_approval_messages_stay_at_current_gate_without_side_effects() {
    let cases = vec![
        (
            "requirements",
            requirements_approval_run(),
            AgentPhase::ConfigureRequirementsApproval,
            ApprovalKind::ConfigureRequirements,
        ),
        (
            "base pack",
            base_pack_approval_run(),
            AgentPhase::ChooseBasePackApproval,
            ApprovalKind::ChooseBasePack,
        ),
        (
            "customization",
            customization_approval_run(),
            AgentPhase::ConfirmCustomizationApproval,
            ApprovalKind::ConfirmCustomization,
        ),
    ];

    for (name, run, expected_phase, expected_kind) in cases {
        for user_message in [
            "I want to go to the beach for coffee.",
            "I want fried rice noodles.",
        ] {
            let original_approval_id = run
                .pending_approval
                .as_ref()
                .expect("case should start at an approval gate")
                .id
                .clone();
            let runtime = approval_route_runtime(serde_json::json!({
                "decision": "needs_clarification",
                "selected_option_id": null,
                "message": null,
                "rationale": "user message is unrelated to the current approval gate"
            }));

            let next = runtime
                .continue_from_user_message(run.clone(), user_message)
                .await
                .unwrap_or_else(|err| {
                    panic!("{name} gate should return a clarification snapshot: {err}")
                });

            assert_eq!(next.status, AgentStatus::WaitingForUser, "{name}");
            assert_eq!(next.phase, expected_phase, "{name}");
            assert_eq!(
                next.pending_approval
                    .as_ref()
                    .map(|approval| &approval.kind),
                Some(&expected_kind),
                "{name}"
            );
            assert_eq!(
                next.pending_approval.as_ref().map(|approval| &approval.id),
                Some(&original_approval_id),
                "{name}"
            );
            assert!(
                next.approved_build.is_none(),
                "{name} gate must not create an approved build"
            );
            assert!(
                next.execution.is_none(),
                "{name} gate must not start execution metadata"
            );
            let last = next
                .messages
                .last()
                .expect("clarification should be recorded as a user-visible message");
            assert_eq!(last.kind, AgentMessageKind::Assistant, "{name}");
            assert!(
                last.text.contains("does not match")
                    && last.text.contains("state was left unchanged"),
                "{name} clarification message should explain the invalid input: {}",
                last.text
            );
        }
    }
}

#[test]
fn search_queries_parse_and_clean_json_output() {
    let cases = [
        (
            r#"{"queries":["query one","query two","query three"]}"#,
            vec!["query one", "query two", "query three"],
        ),
        (
            r#"{"queries":["Create a base modpack search with these queries:","query one","query two"]}"#,
            vec!["query one", "query two"],
        ),
        (
            r#"{"queries":["3D Skin Layers","1. Better Dungeons","- Map Atlases"]}"#,
            vec!["3D Skin Layers", "Better Dungeons", "Map Atlases"],
        ),
    ];

    for (input, expected) in cases {
        let queries = search_queries(input).expect("search-query JSON should parse");
        assert_eq!(queries, expected);
    }
}

#[test]
fn update_build_restrictions_tool_spec_uses_derived_schemas() {
    let spec = update_build_restrictions_tool_spec();
    let expected_input = serde_json::to_value(schemars::schema_for!(UpdateBuildRestrictionsInput))
        .expect("input schema should serialize");
    let expected_output =
        serde_json::to_value(schemars::schema_for!(UpdateBuildRestrictionsOutput))
            .expect("output schema should serialize");

    assert_eq!(spec.input_schema, expected_input);
    assert_eq!(spec.output_schema, expected_output);
    assert_eq!(
        spec.input_schema.get("type").and_then(|v| v.as_str()),
        Some("object")
    );
    let properties = spec
        .input_schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("input schema should expose object properties");
    assert!(properties.contains_key("base_revision"));
    assert!(properties.contains_key("patch"));
}

#[test]
fn modpack_build_react_prompt_and_tools_do_not_expose_state_progression() {
    let prompt = modpack_build_react_prompt();
    let prompt_lc = prompt.to_ascii_lowercase();
    assert!(prompt.contains("ReAct"));
    assert!(prompt.contains("runtime interrupt"));
    assert!(prompt.contains("Do not emit internal progress-control fields"));
    assert!(prompt.contains("Do not force extra mods"));
    assert!(prompt.contains("If a base modpack already satisfies the request"));
    assert!(!prompt_lc.contains("return an object matching"));
    assert!(!prompt_lc.contains("schema requested"));
    assert!(!prompt_lc.contains("typed schema"));
    assert!(!prompt_lc.contains("state machine"));
    assert!(!prompt_lc.contains("hitl_request"));

    let specs = modpack_build_react_tool_specs();
    let names = specs
        .iter()
        .map(|spec| spec.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"update_build_restrictions"));
    assert!(names.contains(&"extract_modpack_goals"));
    assert!(names.contains(&"modpack_search"));
    assert!(names.contains(&"modpack_get_detail"));
    assert!(names.contains(&"mod_search"));
    assert!(names.contains(&"mod_get_detail"));
    assert!(names.contains(&"compatibility_check"));
    assert!(!names.contains(&"select_minecraft_version"));
    assert!(!names.contains(&"hitl_request"));
    assert!(!names.contains(&"request_user_approval"));

    assert!(MODPACK_BUILD_TOOL_LOOP_PROMPT.contains("\"action\":\"request_input\""));
    assert!(MODPACK_BUILD_TOOL_LOOP_PROMPT.contains("select_minecraft_version"));

    for spec in specs {
        let searchable = format!(
            "{} {} {} {}",
            spec.name, spec.description, spec.input_schema, spec.output_schema
        )
        .to_ascii_lowercase();
        assert!(!searchable.contains("next_state"), "{searchable}");
        assert!(!searchable.contains("should_advance"), "{searchable}");
        assert!(!searchable.contains("advance_to"), "{searchable}");
        assert!(!searchable.contains("phase"), "{searchable}");
        assert!(!spec.name.contains("advance"), "{}", spec.name);
    }
}
