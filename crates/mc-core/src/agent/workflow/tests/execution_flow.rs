use super::*;

#[test]
fn resolved_mod_payload_contains_source_metadata_not_execution_manifest() {
    let hit = test_search_hit("mod-project", "Mod Title");
    let mut file = test_version_file("mod-project");
    file.filename = "mod.jar".to_string();
    file.client_side = ProjectSideSupport::Unknown;
    file.server_side = ProjectSideSupport::Unknown;
    let resolved = ResolvedModCandidate {
        candidate: ModCandidate {
            provider: ProviderId::Modrinth,
            hit,
            matched_query: "exploration".to_string(),
        },
        version: crate::modplatform::ProjectVersion {
            id: "version-id".to_string(),
            name: "Version Name".to_string(),
            version_number: "1.0.0".to_string(),
            game_versions: vec!["1.20.1".to_string()],
            loaders: vec!["fabric".to_string()],
            files: vec![file.clone()],
            dependencies: Vec::new(),
            client_side: ProjectSideSupport::Unknown,
            server_side: ProjectSideSupport::Unknown,
        },
        file,
    };

    let value = resolved_mod_payload(&resolved);

    assert_eq!(
        value
            .get("resolved_version")
            .and_then(|v| v.get("version_id"))
            .and_then(|v| v.as_str()),
        Some("version-id")
    );
    assert_eq!(
        value
            .get("resolved_version")
            .and_then(|v| v.get("primary_file"))
            .and_then(|v| v.get("filename"))
            .and_then(|v| v.as_str()),
        Some("mod.jar")
    );
    assert!(value.get("mrpack_file").is_none());
    assert!(value.get("execution_source").is_none());
    assert_eq!(
        value
            .get("source_ref")
            .and_then(|v| v.get("file"))
            .and_then(|v| v.get("filename"))
            .and_then(|v| v.as_str()),
        Some("mod.jar")
    );
    assert_eq!(
        value.get("review_reason").and_then(|v| v.as_str()),
        Some("matched exploration")
    );
    assert_eq!(
        value.get("review_source").and_then(|v| v.as_str()),
        Some("selected_candidate")
    );
    assert_eq!(
        value.get("review_file").and_then(|v| v.as_str()),
        Some("mod.jar")
    );
    assert_eq!(
        value.get("review_version").and_then(|v| v.as_str()),
        Some("1.0.0")
    );
}

#[test]
fn mrpack_file_payload_sanitizes_provider_filename() {
    let mut file = test_version_file("mod-project");
    file.filename = "../nested/evil.jar".to_string();
    file.client_side = ProjectSideSupport::Unknown;
    file.server_side = ProjectSideSupport::Unknown;

    let payload = mrpack_file_payload(&file).expect("remote file should be eligible");
    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .expect("remote file path should be present");

    assert_eq!(path, "mods/evil.jar");
    assert!(!path.contains(".."));
    assert!(!path["mods/".len()..].contains('/'));
    assert!(!path["mods/".len()..].contains('\\'));
}

#[test]
fn mrpack_file_payload_maps_project_side_env() {
    let cases = [
        (
            ProjectSideSupport::Required,
            ProjectSideSupport::Unsupported,
            "required",
            "unsupported",
        ),
        (
            ProjectSideSupport::Unknown,
            ProjectSideSupport::Unknown,
            "optional",
            "optional",
        ),
    ];

    for (client_side, server_side, expected_client, expected_server) in cases {
        let mut file = test_version_file("mod-project");
        file.client_side = client_side;
        file.server_side = server_side;

        let payload = mrpack_file_payload(&file).expect("remote payload should compile");

        assert_eq!(
            payload
                .get("env")
                .and_then(|v| v.get("client"))
                .and_then(|v| v.as_str()),
            Some(expected_client)
        );
        assert_eq!(
            payload
                .get("env")
                .and_then(|v| v.get("server"))
                .and_then(|v| v.as_str()),
            Some(expected_server)
        );
    }
}

#[test]
fn exec_compile_metadata_merges_base_index_and_extra_mod_refs() {
    let base_index = test_mrpack_index(vec![test_mrpack_file(
        "mods/base.jar",
        Some((EnvSupport::Required, EnvSupport::Required)),
    )]);
    let mut extra_file = test_version_file("extra-project");
    extra_file.client_side = ProjectSideSupport::Unknown;
    extra_file.server_side = ProjectSideSupport::Unknown;
    let approved = test_approved_build(vec![test_extra_mod_ref(
        "Extra Mod",
        "extra-project",
        &extra_file,
    )]);

    let metadata = compile_mrpack_execution_metadata(
        &approved,
        &base_index,
        &std::collections::HashMap::new(),
    )
    .unwrap();

    assert_eq!(
        metadata.get("status").and_then(|v| v.as_str()),
        Some("ready")
    );
    assert_eq!(
        metadata
            .get("output_index")
            .and_then(|v| v.get("files"))
            .and_then(|v| v.as_array())
            .map(Vec::len),
        Some(2)
    );
    let extra_remote_files = metadata
        .get("extra_remote_files")
        .and_then(|v| v.as_array())
        .expect("extra remote files should be listed");
    assert_eq!(extra_remote_files.len(), 1);
    assert_eq!(
        extra_remote_files[0]
            .get("project_side")
            .and_then(|v| v.get("fallback"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        extra_remote_files[0]
            .get("env_fallback")
            .and_then(|v| v.as_str()),
        Some("unknown project side metadata mapped to optional")
    );
    assert!(
        approved
            .execution_recipe
            .as_ref()
            .and_then(|v| v.get("extra_remote_files"))
            .is_none()
    );
}

#[test]
fn exec_compile_metadata_applies_base_file_env_overrides() {
    use std::collections::HashMap;

    let base_index = test_mrpack_index(vec![
        test_mrpack_file("mods/client.jar", None),
        test_mrpack_file("mods/fallback.jar", None),
        test_mrpack_file(
            "mods/explicit.jar",
            Some((EnvSupport::Optional, EnvSupport::Unsupported)),
        ),
    ]);
    let approved = test_approved_build(Vec::new());
    let env_overrides = HashMap::from([(
        "mods/client.jar".to_string(),
        (
            ProjectSideSupport::Required,
            ProjectSideSupport::Unsupported,
        ),
    )]);

    let metadata =
        compile_mrpack_execution_metadata(&approved, &base_index, &env_overrides).unwrap();
    let files = metadata
        .get("output_index")
        .and_then(|v| v.get("files"))
        .and_then(|v| v.as_array())
        .expect("output files should be present");
    let env_for = |path: &str| {
        files
            .iter()
            .find(|file| file.get("path").and_then(|v| v.as_str()) == Some(path))
            .and_then(|file| file.get("env"))
            .cloned()
            .expect("file env should be present")
    };

    assert_eq!(
        env_for("mods/client.jar"),
        serde_json::json!({ "client": "required", "server": "unsupported" })
    );
    assert_eq!(
        env_for("mods/fallback.jar"),
        serde_json::json!({ "client": "required", "server": "required" })
    );
    assert_eq!(
        env_for("mods/explicit.jar"),
        serde_json::json!({ "client": "optional", "server": "unsupported" })
    );
}

#[test]
fn exec_compile_metadata_sanitizes_override_source_paths() {
    let base_index = test_mrpack_index(Vec::new());
    let extra_file = test_override_version_file("extra-project", "..\\nested\\evil.jar");
    let approved = test_approved_build(vec![test_extra_mod_ref(
        "Extra Mod",
        "extra-project",
        &extra_file,
    )]);

    let metadata = compile_mrpack_execution_metadata(
        &approved,
        &base_index,
        &std::collections::HashMap::new(),
    )
    .unwrap();
    let source = metadata
        .get("extra_override_sources")
        .and_then(|v| v.as_array())
        .and_then(|v| v.first())
        .expect("override source should be present");

    assert_eq!(
        source.get("install_path").and_then(|v| v.as_str()),
        Some("mods/evil.jar")
    );
    assert_eq!(
        source.get("archive_path").and_then(|v| v.as_str()),
        Some("overrides/mods/evil.jar")
    );
}

#[test]
fn mrpack_build_skips_base_override_entries_that_conflict_with_indexed_files() {
    let base_index = test_mrpack_index(vec![test_mrpack_file(
        "mods/journeymap.jar",
        Some((EnvSupport::Optional, EnvSupport::Optional)),
    )]);
    let base_index_json = serde_json::to_vec(&base_index).unwrap();
    let base_archive = zip_bytes(&[
        ("modrinth.index.json", base_index_json.as_slice()),
        ("overrides/config/journeymap-server.json", b"{}".as_slice()),
        (
            "overrides/mods/journeymap.jar",
            b"duplicate jar override".as_slice(),
        ),
    ]);
    let approved = test_approved_build(Vec::new());

    let built = build_mrpack_from_base_archive_bytes(&approved, &base_archive, &[]).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(built.archive_bytes)).unwrap();

    assert!(
        archive.by_name("overrides/mods/journeymap.jar").is_err(),
        "base override jars that duplicate indexed files must be dropped"
    );
    assert!(
        archive
            .by_name("overrides/config/journeymap-server.json")
            .is_ok(),
        "non-conflicting base overrides must still be preserved"
    );
}

#[test]
fn exec_compile_metadata_blocks_unverifiable_override_source() {
    let base_index = test_mrpack_index(Vec::new());
    let mut extra_file = test_override_version_file("extra-project", "extra.jar");
    extra_file.sha1 = None;
    extra_file.sha512 = None;
    extra_file.size = None;
    let approved = test_approved_build(vec![test_extra_mod_ref(
        "Unverified Override",
        "extra-project",
        &extra_file,
    )]);

    let base_index_json = serde_json::to_vec(&base_index).unwrap();
    let base_archive = zip_bytes(&[("modrinth.index.json", &base_index_json)]);
    let built = build_mrpack_from_base_archive_bytes(&approved, &base_archive, &[]).unwrap();

    assert_eq!(
        built.manifest.get("status").and_then(|v| v.as_str()),
        Some("blocked")
    );
    assert_eq!(
        built.manifest.get("replan_phase").and_then(|v| v.as_str()),
        Some("confirm_customization_approval")
    );
    let blocked = built
        .manifest
        .get("blocked")
        .and_then(|v| v.as_array())
        .expect("blocked manifest should list blocked files");
    assert!(blocked.iter().any(|item| {
        item.get("reason").and_then(|v| v.as_str())
            == Some("override source has no verifiable hash")
    }));
    assert!(
        built.archive_bytes.is_empty(),
        "blocked override sources must not be packaged"
    );
}

#[test]
fn execution_ready_and_completed_manifests_route_forward() {
    let cases = [
        (
            AgentPhase::ExecutionReady,
            serde_json::json!({
                "status": "ready",
                "format": "mrpack",
                "output_index": { "files": [] }
            }),
            AgentStatus::Running,
            AgentPhase::Executing,
            AgentExecutionStatus::Ready,
        ),
        (
            AgentPhase::Executing,
            serde_json::json!({
                "status": "completed",
                "output_path": "/tmp/pack.mrpack"
            }),
            AgentStatus::Completed,
            AgentPhase::Completed,
            AgentExecutionStatus::Completed,
        ),
    ];

    for (initial_phase, manifest, status, phase, execution_status) in cases {
        let next = continue_after_execution_manifest_result(
            execution_manifest_run(initial_phase),
            manifest,
        )
        .expect("execution manifest should advance");

        assert_eq!(next.status, status);
        assert_eq!(next.phase, phase);
        assert!(next.pending_approval.is_none());
        assert_eq!(
            next.execution.as_ref().map(|e| &e.status),
            Some(&execution_status)
        );
    }
}

#[test]
fn execution_manifest_blocked_returns_to_replan_gate() {
    let cases = [
        (
            "confirm_customization_approval",
            "Extra Mod",
            "missing resolved source file",
            AgentPhase::ConfirmCustomizationApproval,
            ApprovalKind::ConfirmCustomization,
            false,
        ),
        (
            "base_pack",
            "Base Pack",
            "base archive missing modrinth.index.json",
            AgentPhase::ChooseBasePackApproval,
            ApprovalKind::ChooseBasePack,
            false,
        ),
        (
            "requirements",
            "target",
            "selected loader is incompatible with requested version",
            AgentPhase::ConfigureRequirementsApproval,
            ApprovalKind::ConfigureRequirements,
            true,
        ),
    ];

    for (replan_phase, title, reason, expected_phase, expected_kind, needs_restrictions) in cases {
        let mut run = AgentRunSnapshot::new("make a pack");
        run.status = AgentStatus::Running;
        run.phase = AgentPhase::ExecutionReady;
        if needs_restrictions {
            run.restrictions = Some(BuildRestrictions {
                minecraft_version: Some("1.20.1".to_string()),
                loader: Some("fabric".to_string()),
                ..BuildRestrictions::default()
            });
        } else {
            run.approved_build = Some(test_approved_build(Vec::new()));
        }

        let next = continue_after_execution_manifest_result(
            run,
            serde_json::json!({
                "status": "blocked",
                "replan_phase": replan_phase,
                "blocked": [{ "title": title, "reason": reason }]
            }),
        )
        .expect("blocked manifest should return to a HITL gate");

        assert_eq!(next.status, AgentStatus::WaitingForUser);
        assert_eq!(next.phase, expected_phase);
        assert_eq!(
            next.execution.as_ref().map(|e| &e.status),
            Some(&AgentExecutionStatus::Blocked)
        );
        let approval = next.pending_approval.expect("approval should be restored");
        assert_eq!(approval.kind, expected_kind);
        assert!(approval.message.contains(reason));
    }
}

#[tokio::test]
async fn final_customization_approval_can_continue_without_model() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.pending_approval = Some(ApprovalRequest {
        id: "approval-test".to_string(),
        kind: ApprovalKind::ConfirmCustomization,
        title: "Confirm customization plan".to_string(),
        message: "Execute after confirmation".to_string(),
        options: vec![ApprovalOption {
            id: "confirm:recommended_customization".to_string(),
            label: "Confirm recommended plan".to_string(),
            description: None,
            payload: Some(serde_json::json!({
                "base_pack": { "title": "Base Pack" },
                "target": { "minecraft_version": "1.20.1", "loader": "fabric" },
                "extra_mods": [],
                "execution_recipe": {
                    "format": "mrpack",
                    "kind": "mrpack_from_base_modpack"
                }
            })),
        }],
        available_decisions: approval_decisions("Confirm recommended plan", "Change extra mods"),
        tools: Vec::new(),
        plan: None,
    });
    let decision = UserDecision {
        approval_id: "approval-test".to_string(),
        kind: UserDecisionKind::Approve,
        selected_option_id: Some("confirm:recommended_customization".to_string()),
        message: None,
        edits: serde_json::Value::Null,
    };

    let runtime = test_main_runtime();
    let next = runtime
        .continue_modpack_build(run, decision)
        .await
        .expect("final approval should complete without another model turn");

    assert_eq!(next.status, AgentStatus::Running);
    assert_eq!(next.phase, AgentPhase::ExecutionReady);
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::NotStarted)
    );
    let build = next
        .approved_build
        .expect("approved build should be present");
    assert_eq!(
        build
            .execution_recipe
            .as_ref()
            .and_then(|v| v.get("format"))
            .and_then(|v| v.as_str()),
        Some("mrpack")
    );
}

#[test]
fn customization_back_records_feedback_for_agent_loop() {
    let base = test_selected_base_pack();
    let target = TargetCompatibility {
        minecraft_version: Some("1.20.1".to_string()),
        loader: Some("fabric".to_string()),
        version_id: Some("base-version".to_string()),
        version_name: Some("Base Version".to_string()),
        version_number: Some("1.0.0".to_string()),
        game_versions: vec!["1.20.1".to_string()],
        loaders: vec!["fabric".to_string()],
        primary_file: None,
        dependencies: Vec::new(),
    };
    let (_, approval) = customization_approval(
        "make a pack",
        &base,
        &target,
        test_base_pack_payload(1),
        Vec::new(),
    );
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.pending_approval = Some(approval.clone());
    run.approved_build = Some(test_approved_build(Vec::new()));
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::NotStarted,
        manifest: None,
        blocked: None,
    });
    let decision = UserDecision {
        approval_id: approval.id,
        kind: UserDecisionKind::Approve,
        selected_option_id: Some("back:choose_base_pack".to_string()),
        message: None,
        edits: serde_json::Value::Null,
    };

    let approval = run.pending_approval.clone().unwrap();
    let next = apply_modpack_build_user_decision(run, &approval, decision)
        .expect("back action should be recorded for the agent loop");

    assert_eq!(next.status, AgentStatus::Running);
    assert!(next.pending_approval.is_none());
    assert!(next.agent_memory.get("latest_feedback").is_some());
}

#[test]
fn blocked_customization_draft_does_not_advertise_unusable_revise() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    let blocked = CustomizationPlanningBlocked {
        reason: "customization planning reached max iterations without a validated plan"
            .to_string(),
        replan_phase: AgentPhase::ConfirmCustomizationApproval,
        details: serde_json::json!({ "max_iterations": 5 }),
    };

    let next = block_customization_planning(run, blocked);
    assert_eq!(next.status, AgentStatus::Running);
    assert!(next.pending_approval.is_none());
    let approval = approval_draft(&next, "confirm_customization_approval");

    assert!(
        !approval
            .available_decisions
            .iter()
            .any(|d| d.kind == UserDecisionKind::Revise),
        "blocked customization gate must not advertise a revise path without a recommended customization payload"
    );
}

#[test]
fn blocked_customization_back_records_feedback_for_agent_loop() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    let base_pack = test_base_pack_payload(7);
    let blocked = CustomizationPlanningBlocked {
        reason: "mod planning reached round cap 6 with unresolved goals".to_string(),
        replan_phase: AgentPhase::ConfirmCustomizationApproval,
        details: serde_json::json!({
            "round": 6,
            "base_pack": base_pack,
        }),
    };

    let blocked_run = block_customization_planning(run, blocked);
    assert_eq!(blocked_run.status, AgentStatus::Running);
    assert!(blocked_run.pending_approval.is_none());
    let approval = approval_draft(&blocked_run, "confirm_customization_approval");
    let back = approval
        .options
        .iter()
        .find(|option| option.id == "back:choose_base_pack")
        .expect("blocked approval should offer back to base-pack selection");
    assert_eq!(
        back.payload
            .as_ref()
            .and_then(|payload| payload.get("base_pack"))
            .and_then(|base_pack| base_pack.get("project_id"))
            .and_then(|project_id| project_id.as_str()),
        Some("base-project-7")
    );
    let decision = UserDecision {
        approval_id: approval.id.clone(),
        kind: UserDecisionKind::Approve,
        selected_option_id: Some("back:choose_base_pack".to_string()),
        message: None,
        edits: serde_json::Value::Null,
    };

    let next = apply_modpack_build_user_decision(blocked_run, &approval, decision)
        .expect("blocked back action should be recorded for the agent loop");

    assert_eq!(next.status, AgentStatus::Running);
    assert!(next.pending_approval.is_none());
    assert!(next.agent_memory.get("latest_feedback").is_some());
}

#[tokio::test]
async fn artifact_tool_executes_approved_execution_ready_run_to_completed() {
    let base_archive = base_archive_for_artifact_tool();
    let base_url = one_response_server(base_archive.clone());
    let run = execution_ready_run(
        &format!("{base_url}/base.mrpack"),
        base_archive.len() as u64,
    );
    let tool = run
        .tools
        .iter()
        .find(|tool| tool.name == "export_mrpack_artifact")
        .expect("execution-ready run should expose deterministic export tool");
    assert_eq!(
        tool.input_schema
            .get("properties")
            .and_then(|v| v.get("output_path"))
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str()),
        Some("string")
    );
    let output = temp_mrpack_path("artifact-tool-completed");
    let runtime = test_main_runtime();

    let next = runtime
        .execute_tool(
            run,
            EXPORT_MRPACK_ARTIFACT_TOOL,
            serde_json::json!({ "output_path": output.to_string_lossy().to_string() }),
        )
        .await
        .expect("artifact tool should drive deterministic execution");

    assert_eq!(next.status, AgentStatus::Completed);
    assert_eq!(next.phase, AgentPhase::Completed);
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::Completed)
    );
    assert!(
        next.trace
            .iter()
            .any(|event| event.event.contains("entering verifying phase"))
    );
    assert!(
        next.trace
            .iter()
            .any(|event| event.event.contains("verification completed"))
    );
    assert!(
        output.exists(),
        "artifact tool should write the mrpack artifact"
    );
    let _ = std::fs::remove_file(output);
}
