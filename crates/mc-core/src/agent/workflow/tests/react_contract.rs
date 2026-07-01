use super::*;

#[test]
fn modpack_build_runtime_no_longer_uses_structured_schema_outputs() {
    let sources = [
        ("workflow.rs", include_str!("../../workflow.rs")),
        ("requirements.rs", include_str!("../requirements.rs")),
        ("base_search.rs", include_str!("../base_search.rs")),
        ("customization.rs", include_str!("../customization.rs")),
    ];

    for (name, source) in sources {
        assert!(
            !source.contains("prompt_typed::<"),
            "{name} still routes modpackbuild through provider structured-output schemas"
        );
        assert!(
            !source.contains("Return an object matching the provided schema"),
            "{name} still asks the model to advance by returning a schema object"
        );
        assert!(
            !source.contains("provided tool input schema"),
            "{name} still exposes old schema-return prompt wording"
        );
    }
}

#[test]
fn modpack_build_runtime_no_longer_uses_staged_workflow_runner() {
    let workflow_source = include_str!("../../workflow.rs");
    let mod_source = include_str!("../../mod.rs");

    assert!(
        !workflow_source.contains("pub struct ModpackBuildWorkflow"),
        "modpack_build must be a prompt-guided tool loop, not a Rust staged workflow runner"
    );
    assert!(
        !workflow_source.contains("modpack_build: ModpackBuildWorkflow"),
        "MainAgentRuntime must not own a staged modpack_build workflow"
    );
    assert!(
        !workflow_source.contains("advance_with_executor"),
        "execution/export must be exposed as tools, not a phase-driven runtime advance loop"
    );
    assert!(
        !mod_source.contains("ModpackBuildWorkflow"),
        "public agent API must not re-export the old staged workflow type"
    );
}

#[test]
fn modpack_build_react_runner_initializes_snapshot_contract() {
    let run = begin_modpack_build_react_run("make a Create Fabric 1.20.1 pack");

    assert_eq!(run.status, AgentStatus::Running);
    assert_eq!(run.phase, AgentPhase::IntentExtraction);
    assert!(run.pending_approval.is_none());
    assert!(run.tools.iter().any(|tool| tool.name == "modpack_search"));
    assert_eq!(run.messages.len(), 1);
    assert_eq!(run.messages[0].kind, AgentMessageKind::User);
    assert!(run.trace.iter().any(|event| {
        event.stream_kind == Some(AgentStreamEventKind::Milestone)
            && event.event.contains("prompt-guided ReAct runner")
    }));
}

#[test]
fn extracted_modpack_goals_feed_planning_feature_tags() {
    let mut run = AgentRunSnapshot::new("make a Fabric 1.20.1 pack with minimap and inventory");
    run.status = AgentStatus::Running;
    run.restrictions = Some(BuildRestrictions {
        revision: 1,
        minecraft_version: Some("1.20.1".to_string()),
        minecraft_version_requirement: Some("1.20.1".to_string()),
        loader: Some("fabric".to_string()),
        feature_tags: vec!["exploration".to_string()],
        notes: Some("original notes".to_string()),
        history: Vec::new(),
    });

    let next = apply_extracted_modpack_goals(
        run,
        serde_json::json!({
            "goals": ["minimap", "inventory management", "minimap"],
            "rationale": "explicit user feature requirements"
        }),
        2,
    )
    .expect("goal extraction tool should update planning feature tags");

    let restrictions = next
        .restrictions
        .expect("goal extraction should keep restrictions");
    assert_eq!(
        restrictions.feature_tags,
        vec!["minimap".to_string(), "inventory management".to_string()]
    );
    assert_eq!(restrictions.revision, 2);
    assert_eq!(
        next.agent_memory
            .get("modpack_goals")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(2)
    );
    let goal_trace = next
        .trace
        .iter()
        .find(|trace| trace.tool.as_deref() == Some("extract_modpack_goals"))
        .expect("goal extraction should emit a tool trace");
    assert_eq!(
        goal_trace
            .output
            .as_ref()
            .and_then(|value| value.get("goals"))
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(2)
    );
}

#[test]
fn version_selection_interrupt_exposes_picker_payload() {
    let mut run = AgentRunSnapshot::new("make a Fabric 1.20.x exploration pack");
    run.status = AgentStatus::Running;
    run.restrictions = Some(BuildRestrictions {
        revision: 1,
        minecraft_version: None,
        minecraft_version_requirement: Some("1.20.x".to_string()),
        loader: Some("fabric".to_string()),
        feature_tags: vec!["exploration".to_string()],
        notes: None,
        history: Vec::new(),
    });

    let next = request_modpack_agent_input(
        run,
        ModpackAgentInputKind::SelectMinecraftVersion,
        serde_json::json!({
            "version_request": "1.20.x",
            "candidates": ["1.20.4", "1.20.1"],
            "loader": "fabric"
        }),
    )
    .expect("version selection should pause through a structured interrupt");

    assert_eq!(next.status, AgentStatus::WaitingForUser);
    assert_eq!(next.phase, AgentPhase::ConfigureRequirementsApproval);
    assert!(next.pending_approval.is_none());
    let interrupt = next
        .pending_interrupt
        .expect("version selection should expose a pending interrupt");
    assert_eq!(interrupt.kind, AgentInterruptKind::UserInput);
    assert_eq!(
        interrupt.input_kind,
        Some(AgentInputKind::SelectMinecraftVersion)
    );
    assert_eq!(interrupt.title, "Choose Minecraft version");
    assert!(interrupt.allow_freeform);
    assert_eq!(interrupt.options.len(), 2);
    assert_eq!(interrupt.options[0].id, "1.20.4");
    assert_eq!(interrupt.options[0].label, "1.20.4");
}

#[test]
fn version_selection_resume_updates_build_restrictions() {
    let mut run = AgentRunSnapshot::new("make a Fabric 1.20.x exploration pack");
    run.status = AgentStatus::Running;
    run.restrictions = Some(BuildRestrictions {
        revision: 1,
        minecraft_version: None,
        minecraft_version_requirement: Some("1.20.x".to_string()),
        loader: Some("fabric".to_string()),
        feature_tags: vec!["exploration".to_string()],
        notes: Some("keep lightweight".to_string()),
        history: Vec::new(),
    });
    let run = request_modpack_agent_input(
        run,
        ModpackAgentInputKind::SelectMinecraftVersion,
        serde_json::json!({
            "version_request": "1.20.x",
            "candidates": ["1.20.4", "1.20.1"],
            "loader": "fabric"
        }),
    )
    .expect("version selection should pause");
    let interrupt = run
        .pending_interrupt
        .clone()
        .expect("version selection should be pending");

    let next = apply_modpack_build_user_input(run, &interrupt, "1.20.1")
        .expect("selected version should resume requirements planning");

    assert_eq!(next.status, AgentStatus::Running);
    assert!(next.pending_interrupt.is_none());
    assert!(next.pending_approval.is_none());
    let restrictions = next.restrictions.expect("restrictions should be updated");
    assert_eq!(restrictions.minecraft_version.as_deref(), Some("1.20.1"));
    assert_eq!(
        restrictions.minecraft_version_requirement.as_deref(),
        Some("1.20.1")
    );
    assert_eq!(restrictions.loader.as_deref(), Some("fabric"));
    assert_eq!(restrictions.feature_tags, vec!["exploration".to_string()]);
    assert_eq!(restrictions.notes.as_deref(), Some("keep lightweight"));
    assert_eq!(restrictions.revision, 2);
}

#[tokio::test]
async fn structured_version_input_resume_updates_then_returns_to_agent_loop() {
    let mut run = AgentRunSnapshot::new("make a Fabric 1.20.x exploration pack");
    run.status = AgentStatus::Running;
    run.restrictions = Some(BuildRestrictions {
        revision: 1,
        minecraft_version: None,
        minecraft_version_requirement: Some("1.20.x".to_string()),
        loader: Some("fabric".to_string()),
        feature_tags: vec!["exploration".to_string()],
        notes: None,
        history: Vec::new(),
    });
    let run = request_modpack_agent_input(
        run,
        ModpackAgentInputKind::SelectMinecraftVersion,
        serde_json::json!({
            "version_request": "1.20.x",
            "candidates": ["1.20.4", "1.20.1"],
            "loader": "fabric"
        }),
    )
    .expect("version selection should pause");
    let resume_token = run
        .pending_interrupt
        .as_ref()
        .expect("version input should be pending")
        .resume_token
        .clone();
    let runtime = approval_route_runtime(serde_json::json!({
        "action": "final",
        "message": "done"
    }));

    let next = runtime
        .continue_from_user_input(run, &resume_token, "1.20.1")
        .await
        .expect("structured input should resume through the agent loop");

    let restrictions = next.restrictions.expect("restrictions should be updated");
    assert_eq!(restrictions.minecraft_version.as_deref(), Some("1.20.1"));
    assert_eq!(next.status, AgentStatus::Completed);
    assert!(next.pending_interrupt.is_none());
}

#[test]
fn parses_restriction_tool_input_without_inventing_missing_target() {
    let input = parse_restriction_update_response(
            r#"{"base_revision":0,"patch":{"minecraft_version":null,"loader":null,"feature_tags":["industrial automation"],"notes":"missing target"}}"#,
        )
        .expect("restriction tool json should parse");

    assert!(input.patch.minecraft_version.is_none());
    assert!(input.patch.loader.is_none());
    let output = update_build_restrictions(
        Some(BuildRestrictions::default()),
        input,
        BuildRestrictionChangeSource::InitialPrompt,
        "initial",
    )
    .expect("restriction update should apply");
    assert_eq!(output.missing_fields, vec!["minecraft_version", "loader"]);
}

#[test]
fn rejects_repeated_restriction_tool_argument_instances() {
    let text = r#"{"base_revision":3,"patch":{"feature_tags":["adventure","exploration","qol"],"loader":"fabric","minecraft_version":"1.20.1","minecraft_version_requirement":"1.20.1","notes":"prefer dungeons and QoL"}}
{"base_revision":3,"patch":{"feature_tags":["adventure","exploration","qol"],"loader":"fabric","minecraft_version":"1.20.1","minecraft_version_requirement":"1.20.1","notes":"prefer dungeons and QoL"}}"#;

    let err = parse_restriction_update_response(text)
        .expect_err("tool argument parser should reject multiple root objects");

    assert!(
        err.to_string()
            .contains("single restriction tool argument object")
    );
}

#[test]
fn restriction_update_llm_payload_omits_restriction_history() {
    let mut restrictions = BuildRestrictions {
        revision: 7,
        minecraft_version: Some("1.20.1".to_string()),
        minecraft_version_requirement: Some("1.20.1".to_string()),
        loader: Some("fabric".to_string()),
        feature_tags: vec!["adventure".to_string()],
        notes: Some("prefer exploration".to_string()),
        history: Vec::new(),
    };
    restrictions.history.push(BuildRestrictionChange {
        revision: 7,
        source: BuildRestrictionChangeSource::UserRevise,
        patch: BuildRestrictionPatch {
            minecraft_version: Some("1.20.1".to_string()),
            minecraft_version_requirement: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
            feature_tags: vec!["adventure".to_string()],
            notes: Some("prefer exploration".to_string()),
        },
        summary: "large audit history entry".to_string(),
    });

    let payload = restriction_update_request_payload(
        "make a pack",
        &restrictions,
        "Change it toward exploration",
        BuildRestrictionChangeSource::UserRevise,
        None,
        None,
    );

    assert_eq!(
        payload.get("base_revision").and_then(|v| v.as_u64()),
        Some(7)
    );
    let current = payload
        .get("current_restrictions")
        .and_then(|v| v.as_object())
        .expect("current restrictions should be an object");
    assert_eq!(
        current.get("minecraft_version").and_then(|v| v.as_str()),
        Some("1.20.1")
    );
    assert!(!current.contains_key("history"), "payload: {payload}");
    assert!(
        !payload.to_string().contains("large audit history entry"),
        "payload: {payload}"
    );
}

#[test]
fn requirements_approval_always_offers_audit_approve_decision() {
    let cases = [
        (
            "make a Fabric 1.20.1 pack",
            r#"{"base_revision":0,"patch":{"minecraft_version":"1.20.1","loader":"fabric","feature_tags":["industrial","automation"],"notes":null}}"#,
            0,
        ),
        (
            "make an adventure pack",
            r#"{"base_revision":0,"patch":{"minecraft_version":null,"loader":null,"feature_tags":["adventure"],"notes":null}}"#,
            2,
        ),
    ];

    for (prompt, input_json, expected_missing) in cases {
        let input = parse_restriction_update_response(input_json)
            .expect("restriction tool json should parse");
        let output = update_build_restrictions(
            Some(BuildRestrictions::default()),
            input,
            BuildRestrictionChangeSource::InitialPrompt,
            "initial",
        )
        .expect("restriction update should apply");
        let approval = requirements_approval(prompt, &output);

        assert_eq!(approval.kind, ApprovalKind::ConfigureRequirements);
        assert!(
            approval
                .available_decisions
                .iter()
                .any(|d| d.kind == UserDecisionKind::Approve)
        );
        assert_eq!(approval.tools[0].name, UPDATE_BUILD_RESTRICTIONS_TOOL);
        assert_eq!(approval.options[0].id, "requirements:detected");
        let missing = approval.options[0]
            .payload
            .as_ref()
            .and_then(|p| p.get("missing_fields"))
            .and_then(|v| v.as_array())
            .map(Vec::len)
            .unwrap_or(0);
        assert_eq!(missing, expected_missing);
    }
}

#[test]
fn preserves_raw_version_requirement_without_concrete_target() {
    let input = parse_restriction_update_response(
            r#"{"base_revision":0,"patch":{"minecraft_version":null,"minecraft_version_requirement":"1.20.x","loader":"fabric","feature_tags":["adventure"],"notes":null}}"#,
        )
        .expect("restriction tool json should parse raw version requirement");
    let output = update_build_restrictions(
        Some(BuildRestrictions::default()),
        input,
        BuildRestrictionChangeSource::InitialPrompt,
        "initial",
    )
    .expect("raw version requirement update should apply");

    assert!(output.restrictions.minecraft_version.is_none());
    assert_eq!(
        output.restrictions.minecraft_version_requirement.as_deref(),
        Some("1.20.x")
    );
    assert_eq!(output.missing_fields, vec!["minecraft_version"]);
}

#[test]
fn requirement_summary_does_not_echo_invalid_version_as_accepted_target() {
    let input = parse_restriction_update_response(
            r#"{"base_revision":0,"patch":{"minecraft_version":"99.99","minecraft_version_requirement":"99.99","loader":"fabric","feature_tags":["adventure"],"notes":null}}"#,
        )
        .expect("restriction tool json should parse invalid raw version requirement");
    let output = update_build_restrictions(
        Some(BuildRestrictions::default()),
        input,
        BuildRestrictionChangeSource::InitialPrompt,
        "initial",
    )
    .expect("restriction update should apply");

    assert!(output.restrictions.minecraft_version.is_none());
    let summary = requirement_summary_message(&output);
    assert!(!summary.contains("99.99"), "unexpected summary: {summary}");
    assert!(
        summary.contains("missing: Minecraft version"),
        "unexpected summary: {summary}"
    );
}

#[test]
fn requested_compatibility_comes_from_build_restrictions() {
    let restrictions = BuildRestrictions {
        revision: 1,
        minecraft_version: Some("1.20.1".to_string()),
        minecraft_version_requirement: Some("1.20.1".to_string()),
        loader: Some("fabric".to_string()),
        feature_tags: vec!["lightweight".to_string(), "adventure".to_string()],
        notes: None,
        history: Vec::new(),
    };
    let target = requested_compatibility_from_restrictions(Some(&restrictions));

    assert_eq!(target.minecraft_version.as_deref(), Some("1.20.1"));
    assert_eq!(target.loader.as_deref(), Some("fabric"));
}

#[test]
fn restriction_update_replaces_older_target() {
    let initial = UpdateBuildRestrictionsInput {
        base_revision: 0,
        patch: BuildRestrictionPatch {
            minecraft_version: Some("1.20.1".to_string()),
            minecraft_version_requirement: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
            feature_tags: vec!["automation".to_string()],
            notes: None,
        },
    };
    let first = update_build_restrictions(
        Some(BuildRestrictions::default()),
        initial,
        BuildRestrictionChangeSource::InitialPrompt,
        "initial",
    )
    .expect("initial restriction update should apply");
    let revised = UpdateBuildRestrictionsInput {
        base_revision: first.restrictions.revision,
        patch: BuildRestrictionPatch {
            minecraft_version: Some("1.19.2".to_string()),
            minecraft_version_requirement: Some("1.19.2".to_string()),
            loader: Some("forge".to_string()),
            feature_tags: vec!["adventure".to_string(), "exploration".to_string()],
            notes: None,
        },
    };
    let second = update_build_restrictions(
        Some(first.restrictions),
        revised,
        BuildRestrictionChangeSource::UserRevise,
        "Change it to Forge 1.19.2 with more adventure and exploration",
    )
    .expect("revised restriction update should apply");
    let target = requested_compatibility_from_restrictions(Some(&second.restrictions));

    assert_eq!(target.minecraft_version.as_deref(), Some("1.19.2"));
    assert_eq!(target.loader.as_deref(), Some("forge"));
    assert_eq!(
        second.restrictions.feature_tags,
        vec!["adventure", "exploration"]
    );
}

#[test]
fn requirements_replan_invalidates_downstream_artifacts() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.approved_build = Some(ApprovedModpackBuild {
        base_pack: serde_json::json!({ "title": "Old Base Pack" }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: Vec::new(),
        execution_recipe: Some(serde_json::json!({ "format": "mrpack" })),
    });
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::Ready,
        manifest: Some(serde_json::json!({ "status": "ready" })),
        blocked: None,
    });

    let output = update_build_restrictions(
        Some(BuildRestrictions {
            revision: 1,
            minecraft_version: Some("1.20.1".to_string()),
            minecraft_version_requirement: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
            feature_tags: vec!["adventure".to_string()],
            notes: None,
            history: Vec::new(),
        }),
        UpdateBuildRestrictionsInput {
            base_revision: 1,
            patch: BuildRestrictionPatch {
                minecraft_version: Some("1.19.2".to_string()),
                minecraft_version_requirement: Some("1.19.2".to_string()),
                loader: Some("fabric".to_string()),
                feature_tags: vec!["adventure".to_string()],
                notes: None,
            },
        },
        BuildRestrictionChangeSource::UserRevise,
        "modA requires 1.19.2",
    )
    .expect("restriction update should apply");

    let next = apply_requirements_replan(
        run,
        output,
        "modA requires a different Minecraft version",
        AgentPhase::ConfirmCustomizationApproval,
    );

    assert_eq!(next.status, AgentStatus::Running);
    assert_eq!(next.phase, AgentPhase::BasePackSearch);
    assert!(next.approved_build.is_none());
    assert!(next.execution.is_none());
    assert!(next.pending_approval.is_none());
    assert_eq!(next.replans.len(), 1);
    assert_eq!(
        next.replans[0].invalidates,
        vec![
            PlanArtifact::BasePack,
            PlanArtifact::ExtraMods,
            PlanArtifact::ApprovedBuild,
            PlanArtifact::ExecutionMetadata
        ]
    );
}

#[test]
fn requirements_replan_prepares_base_search_without_second_confirmation() {
    let mut run = requirements_approval_run();
    run.approved_build = Some(ApprovedModpackBuild {
        base_pack: serde_json::json!({ "title": "Old Base Pack" }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "forge" }),
        extra_mods: Vec::new(),
        execution_recipe: Some(serde_json::json!({ "format": "mrpack" })),
    });
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::Ready,
        manifest: Some(serde_json::json!({ "status": "ready" })),
        blocked: None,
    });

    let output = update_build_restrictions(
        run.restrictions.clone(),
        UpdateBuildRestrictionsInput {
            base_revision: 1,
            patch: BuildRestrictionPatch {
                minecraft_version: None,
                minecraft_version_requirement: Some("1.21.x".to_string()),
                loader: Some("forge".to_string()),
                feature_tags: vec!["deep ocean".to_string(), "create".to_string()],
                notes: Some("1.21.x is sufficient; continue searching".to_string()),
            },
        },
        BuildRestrictionChangeSource::UserRevise,
        "1.21.x is sufficient",
    )
    .expect("restriction update should apply");

    let next = apply_requirements_replan(
        run,
        output,
        "requirements revised: 1.21.x is sufficient",
        AgentPhase::ConfigureRequirementsApproval,
    );

    assert_eq!(next.status, AgentStatus::Running);
    assert_eq!(next.phase, AgentPhase::BasePackSearch);
    assert!(next.pending_approval.is_none());
    assert!(next.approved_build.is_none());
    assert!(next.execution.is_none());
    assert_eq!(
        next.restrictions
            .as_ref()
            .and_then(|restrictions| restrictions.minecraft_version_requirement.as_deref()),
        Some("1.21.x")
    );
}

#[test]
fn customization_target_change_ignores_requirement_relaxation_when_concrete_version_stays() {
    let before = BuildRestrictions {
        minecraft_version: Some("1.20.1".to_string()),
        minecraft_version_requirement: Some("1.20.1".to_string()),
        loader: Some("forge".to_string()),
        ..Default::default()
    };
    let after = BuildRestrictions {
        minecraft_version: Some("1.20.1".to_string()),
        minecraft_version_requirement: Some("1.20.x".to_string()),
        loader: Some("forge".to_string()),
        feature_tags: vec![
            "ocean".to_string(),
            "underwater survival".to_string(),
            "exploration".to_string(),
        ],
        ..Default::default()
    };

    assert!(
        !restriction_target_changed(&before, &after),
        "customization feedback that only broadens the requirement text must stay in modplan"
    );
}

#[test]
fn parses_mod_revision_plan_with_removals() {
    let plan = parse_mod_query_response(
            r#"{"queries":["backpack"],"retain_existing_mods":true,"remove_existing_mod_ids":["journeymap"]}"#,
        )
        .expect("structured mod query output should parse");

    assert_eq!(plan.queries, vec!["backpack"]);
    assert!(plan.retain_existing_mods);
    assert_eq!(plan.remove_existing_mod_ids, vec!["journeymap"]);
}

#[test]
fn remove_existing_mod_payloads_filters_requested_ids() {
    let existing = vec![
        serde_json::json!({
            "provider": "modrinth",
            "project_id": "journeymap",
            "slug": "journeymap",
            "title": "JourneyMap"
        }),
        serde_json::json!({
            "provider": "modrinth",
            "project_id": "backpack",
            "slug": "travelers-backpack",
            "title": "Traveler's Backpack"
        }),
    ];

    let next = remove_existing_mod_payloads(existing, &["journeymap".to_string()]);

    assert_eq!(next.len(), 1);
    assert_eq!(
        next[0].get("project_id").and_then(|v| v.as_str()),
        Some("backpack")
    );
}

#[test]
fn base_pack_and_mod_payloads_include_describe() {
    let hit = test_search_hit("project", "Project Title");

    let base_option = candidate_option(&BasePackCandidate {
        provider: ProviderId::Modrinth,
        hit: hit.clone(),
        matched_query: "tech adventure".to_string(),
        resolved_target: None,
    });
    let mod_value = mod_payload(&ModCandidate {
        provider: ProviderId::Modrinth,
        hit,
        matched_query: "backpack".to_string(),
    });

    assert!(
        base_option
            .payload
            .as_ref()
            .and_then(|p| p.get("describe"))
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.contains("tech adventure"))
    );
    assert!(
        mod_value
            .get("describe")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.contains("backpack"))
    );
}
