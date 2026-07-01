use super::*;

#[tokio::test]
async fn artifact_tool_fails_during_verifying_for_invalid_artifact() {
    let mut run = execution_ready_run("https://example.com/base.mrpack", 10);
    run.phase = AgentPhase::Verifying;
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::Running,
        manifest: Some(serde_json::json!({ "status": "verifying" })),
        blocked: None,
    });
    let output = temp_mrpack_path("artifact-tool-verifying-failed");
    let runtime = test_main_runtime();

    let invalid_index = test_mrpack_index(vec![test_mrpack_file("mods/bad.jar", None)]);
    let invalid_index_json = serde_json::to_vec(&invalid_index).unwrap();
    let archive = zip_bytes(&[("modrinth.index.json", &invalid_index_json)]);
    std::fs::write(&output, archive).unwrap();

    let next = runtime
        .execute_tool(
            run,
            EXPORT_MRPACK_ARTIFACT_TOOL,
            serde_json::json!({ "output_path": output.to_string_lossy().to_string() }),
        )
        .await
        .expect("verification failures should be represented in run state");

    assert_eq!(next.status, AgentStatus::Failed);
    assert_eq!(next.phase, AgentPhase::Failed);
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::Failed)
    );
    assert!(
        next.execution
            .as_ref()
            .and_then(|e| e.blocked.as_ref())
            .map(|blocked| blocked.reason.contains("missing env"))
            .unwrap_or(false)
    );
    assert!(
        next.trace
            .iter()
            .any(|event| event.event.contains("execution failed"))
    );
    let _ = std::fs::remove_file(output);
}

#[test]
fn verify_written_mrpack_rejects_deep_invalid_artifacts() {
    let mut approved = test_approved_build(Vec::new());
    approved.execution_recipe = None;
    let valid_file = test_mrpack_file(
        "mods/valid.jar",
        Some((EnvSupport::Required, EnvSupport::Unsupported)),
    );
    let valid_index = test_mrpack_index(vec![valid_file]);

    let mut no_downloads = valid_index.clone();
    no_downloads.files[0].downloads.clear();
    let mut no_env = valid_index.clone();
    no_env.files[0].env = None;

    let cases = vec![
        (
            "no-downloads",
            no_downloads,
            Vec::new(),
            "missing downloads",
        ),
        ("no-env", no_env, Vec::new(), "missing env"),
        (
            "override-conflict",
            valid_index,
            vec![("overrides/mods/valid.jar", b"conflict".as_slice())],
            "conflicts with indexed file",
        ),
    ];

    for (tag, index, extra_files, reason) in cases {
        let output = temp_mrpack_path(tag);
        let index_json = serde_json::to_vec(&index).unwrap();
        let mut files = vec![("modrinth.index.json", index_json.as_slice())];
        files.extend(extra_files);
        std::fs::write(&output, zip_bytes(&files)).unwrap();

        let err = verify_written_mrpack(&output, &approved)
            .expect_err("invalid artifact should fail verification");
        assert!(
            err.to_string().contains(reason),
            "expected {reason:?}, got {err}"
        );
        let _ = std::fs::remove_file(output);
    }
}

#[test]
fn retry_exhausted_manifest_routes_to_terminal_failed() {
    let base_archive = base_archive_for_artifact_tool();
    let run = execution_ready_run("https://example.com/base.mrpack", base_archive.len() as u64);
    let next = continue_after_execution_manifest_result(
        run,
        execution_retry_exhausted_manifest("source timeout", EXECUTION_MAX_RETRIES),
    )
    .expect("retry-exhausted manifest should return a terminal run");

    assert_eq!(next.status, AgentStatus::Failed);
    assert_eq!(next.phase, AgentPhase::Failed);
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::Failed)
    );
    let reason = next
        .execution
        .as_ref()
        .and_then(|execution| execution.blocked.as_ref())
        .map(|blocked| blocked.reason.as_str());
    assert_eq!(
        reason,
        Some("execution exceeded max retries: source timeout")
    );
}

#[tokio::test]
async fn artifact_tool_stops_at_waiting_for_user_gate_without_executing() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.pending_approval = Some(ApprovalRequest {
        id: "approval-test".to_string(),
        kind: ApprovalKind::ConfirmCustomization,
        title: "Confirm".to_string(),
        message: "Confirm".to_string(),
        options: Vec::new(),
        available_decisions: Vec::new(),
        tools: Vec::new(),
        plan: None,
    });
    let output = temp_mrpack_path("artifact-tool-waiting");
    let runtime = test_main_runtime();

    let next = runtime
        .execute_tool(
            run.clone(),
            EXPORT_MRPACK_ARTIFACT_TOOL,
            serde_json::json!({ "output_path": output.to_string_lossy().to_string() }),
        )
        .await
        .expect("waiting gate should return unchanged");

    assert_eq!(next.status, AgentStatus::WaitingForUser);
    assert_eq!(next.phase, AgentPhase::ConfirmCustomizationApproval);
    assert_eq!(
        next.pending_approval
            .as_ref()
            .map(|approval| &approval.kind),
        Some(&ApprovalKind::ConfirmCustomization)
    );
    assert!(
        !output.exists(),
        "artifact tool must not execute past a HITL gate"
    );
}

#[tokio::test]
async fn artifact_tool_does_not_reexecute_completed_run() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::Completed;
    run.phase = AgentPhase::Completed;
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::Completed,
        manifest: Some(serde_json::json!({ "status": "completed" })),
        blocked: None,
    });
    let trace_len = run.trace.len();
    let output = temp_mrpack_path("artifact-tool-idempotent");
    let runtime = test_main_runtime();

    let next = runtime
        .execute_tool(
            run,
            EXPORT_MRPACK_ARTIFACT_TOOL,
            serde_json::json!({ "output_path": output.to_string_lossy().to_string() }),
        )
        .await
        .expect("completed run should be idempotent");

    assert_eq!(next.status, AgentStatus::Completed);
    assert_eq!(next.phase, AgentPhase::Completed);
    assert_eq!(next.trace.len(), trace_len);
    assert!(!output.exists(), "completed run must not be executed again");
}

#[test]
fn agent_base_pack_execution_support_is_modrinth_only() {
    assert!(base_pack_provider_supported_for_execution(
        ProviderId::Modrinth
    ));
    assert!(!base_pack_provider_supported_for_execution(
        ProviderId::CurseForge
    ));
}

#[tokio::test]
async fn mrpack_execution_blocks_non_modrinth_base_provider() {
    let approved = ApprovedModpackBuild {
        base_pack: serde_json::json!({
            "provider": "curseforge",
            "project_id": "12345",
            "slug": "cf-pack",
            "title": "CurseForge Pack"
        }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: Vec::new(),
        execution_recipe: None,
    };
    let output = std::env::temp_dir().join("mc-agent-non-modrinth-should-not-write.mrpack");

    let manifest = execute_mrpack_build_to_path(&approved, &output)
        .await
        .expect("unsupported provider should become a blocked manifest");

    assert_eq!(
        manifest.get("status").and_then(|v| v.as_str()),
        Some("blocked")
    );
    assert_eq!(
        manifest.get("replan_phase").and_then(|v| v.as_str()),
        Some("choose_base_pack_approval")
    );
    assert!(
        manifest
            .get("reason")
            .and_then(|v| v.as_str())
            .is_some_and(|reason| reason.contains("provider is not supported"))
    );
    assert!(!output.exists());
}

#[tokio::test]
async fn mrpack_execution_writes_scratch_pack_with_only_additions() {
    let mut extra_file = test_version_file("ocean");
    extra_file.server_side = ProjectSideSupport::Unsupported;
    let approved = test_scratch_approved_build(
        "fabric",
        vec![test_extra_mod_ref("Ocean Mod", "ocean", &extra_file)],
    );
    let output = temp_mrpack_path("scratch-exec");

    let manifest = execute_mrpack_build_to_path(&approved, &output)
        .await
        .expect("scratch execution should write an artifact");

    assert_eq!(
        manifest.get("status").and_then(|v| v.as_str()),
        Some("verifying")
    );
    verify_written_mrpack(&output, &approved).unwrap();
    let file = std::fs::File::open(&output).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let index: MrpackIndex = {
        let mut index_file = archive.by_name("modrinth.index.json").unwrap();
        serde_json::from_reader(&mut index_file).unwrap()
    };
    assert_eq!(index.files.len(), 1);
    assert_eq!(index.files[0].path, "mods/ocean.jar");
    assert_eq!(
        index.files[0]
            .env
            .as_ref()
            .map(|env| (env.client, env.server)),
        Some((EnvSupport::Required, EnvSupport::Unsupported))
    );
    assert_eq!(index.dependencies.minecraft.as_deref(), Some("1.20.1"));
    assert!(index.dependencies.fabric_loader.is_some());
    let _ = std::fs::remove_file(output);
}

#[tokio::test]
async fn mrpack_scratch_forge_uses_concrete_loader_dependency() {
    let approved = test_scratch_approved_build("forge", Vec::new());
    let output = temp_mrpack_path("scratch-forge-exec");

    execute_mrpack_build_to_path(&approved, &output)
        .await
        .expect("scratch forge execution should write an artifact");

    verify_written_mrpack(&output, &approved).unwrap();
    let file = std::fs::File::open(&output).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let index: MrpackIndex = {
        let mut index_file = archive.by_name("modrinth.index.json").unwrap();
        serde_json::from_reader(&mut index_file).unwrap()
    };
    let forge = index
        .dependencies
        .forge
        .as_deref()
        .expect("forge dependency should be present");
    assert_ne!(forge, "latest");
    assert!(forge.chars().any(|c| c.is_ascii_digit()));
    let _ = std::fs::remove_file(output);
}

#[test]
fn modplan_query_cleanup_strips_filters_and_compatibility_clauses() {
    let candidate_project_ids = std::collections::HashSet::new();
    let goal_ids = ["goal:portals".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let step = ModPlanStep {
        selections: Vec::new(),
        removals: Vec::new(),
        next_queries: vec![GoalQuery {
            goal_id: "goal:portals".to_string(),
            query: "Immersive Portals Fabric 1.20.1 compatibility with SpaceCraft Pluto"
                .to_string(),
        }],
        rationale: String::new(),
    };

    let normalized = step.normalized(&candidate_project_ids, &goal_ids);

    assert_eq!(normalized.next_queries.len(), 1);
    assert_eq!(normalized.next_queries[0].query, "Immersive Portals");
}

#[test]
fn modplan_parser_assigns_default_goal_to_unlabeled_next_queries() {
    let candidate_project_ids = ["P0Mu4wcQ".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let goal_ids = ["theme:minimap".to_string()]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let step = parse_mod_plan_step_response(
        r#"{
            "selections": ["P0Mu4wcQ"],
            "removals": [],
            "next_queries": [
                {"query": "Xaero's Minimap"},
                "Inventory Profiles Next Fabric"
            ],
            "rationale": "Need dedicated minimap and inventory management mods."
        }"#,
        &candidate_project_ids,
        &goal_ids,
        Some("theme:minimap"),
    )
    .expect("missing goal_id in next_queries should use the current pending goal");

    assert_eq!(step.selections.len(), 1);
    assert_eq!(step.selections[0].goal_id, "theme:minimap");
    assert_eq!(step.selections[0].project_id, "P0Mu4wcQ");
    assert_eq!(step.next_queries.len(), 2);
    assert!(
        step.next_queries
            .iter()
            .all(|query| query.goal_id == "theme:minimap")
    );
    assert_eq!(step.next_queries[0].query, "Xaero's Minimap");
    assert_eq!(step.next_queries[1].query, "Inventory Profiles Next");
}

#[test]
fn modplan_fallback_queries_prefer_short_project_or_topic_terms() {
    let queries = fallback_mod_search_queries(&[
        GoalQuery {
            goal_id: "goal:portals".to_string(),
            query: "Please add Immersive Portals to the extra mods, if it is compatible with the current Fabric 1.20.1 base pack.".to_string(),
        },
        GoalQuery {
            goal_id: "goal:qol".to_string(),
            query: "Fabric 1.20.1 quality of life mods compatible with SpaceCraft Pluto base pack realistic portals mod".to_string(),
        },
    ]);

    assert_eq!(
        queries,
        vec![
            "Immersive Portals".to_string(),
            "quality of life".to_string()
        ]
    );
}

#[test]
fn modplan_validation_reports_open_theme_goals_with_model_diagnosis() {
    let (_, _, mut state) = initialized_mod_plan_without_restrictions();
    merge_feedback_into_mod_plan(
        &mut AgentRunSnapshot {
            mod_plan: Some(state.clone()),
            ..AgentRunSnapshot::new("make a pack")
        },
        "Add Advent of Ascension 3",
    );
    state.goals.push(Goal {
        id: "theme:aoa3".to_string(),
        label: "Add Advent of Ascension 3".to_string(),
        kind: GoalKind::Theme,
        status: GoalStatus::Open,
    });

    let unresolved = unresolved_mod_plan_goals(
        &state,
        Some("No compatible Fabric 1.20.1 candidates were available.".to_string()),
    );

    assert!(unresolved.iter().any(|item| {
        item.get("label").and_then(|v| v.as_str()) == Some("Add Advent of Ascension 3")
            && item
                .get("diagnosis")
                .and_then(|v| v.as_str())
                .is_some_and(|text| text.contains("No compatible Fabric"))
    }));
}

#[test]
fn mrpack_executor_rewrites_index_and_preserves_base_overrides() {
    use crate::modpack::formats::mrpack::{
        MrpackDependencies, MrpackFile, MrpackHashes, MrpackIndex,
    };

    let base_index = MrpackIndex {
        format_version: 1,
        game: "minecraft".to_string(),
        version_id: "base-1.0.0".to_string(),
        name: "Base Pack".to_string(),
        summary: None,
        dependencies: MrpackDependencies {
            minecraft: Some("1.20.1".to_string()),
            fabric_loader: Some("0.15.7".to_string()),
            ..Default::default()
        },
        files: vec![MrpackFile {
            path: "mods/base.jar".to_string(),
            hashes: MrpackHashes {
                sha512: "base-sha512".to_string(),
                sha1: None,
            },
            env: None,
            downloads: vec!["https://cdn.modrinth.com/data/base/versions/v/base.jar".to_string()],
            file_size: Some(100),
        }],
    };
    let base_index_json = serde_json::to_vec(&base_index).unwrap();
    let base_archive = zip_bytes_with_dirs(
        &["overrides/", "overrides/config/"],
        &[
            ("modrinth.index.json", &base_index_json),
            ("overrides/config/base.toml", b"keep = true"),
        ],
    );
    let extra_file = VersionFile {
        url: "https://cdn.modrinth.com/data/extra/versions/v/extra.jar".to_string(),
        filename: "extra.jar".to_string(),
        sha1: Some("extra-sha1".to_string()),
        sha512: Some("extra-sha512".to_string()),
        size: Some(200),
        primary: true,
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    };
    let approved = ApprovedModpackBuild {
        base_pack: serde_json::json!({ "title": "Base Pack" }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: Vec::new(),
        execution_recipe: Some(serde_json::json!({
            "schema_version": 1,
            "kind": "mrpack_from_base_modpack",
            "format": "mrpack",
            "extra_mod_refs": [{
                "title": "Extra Mod",
                "project_id": "extra-project",
                "source_ref": {
                    "kind": "mod_file",
                    "provider": "modrinth",
                    "project_id": "extra-project",
                    "version_id": "extra-version",
                    "file": version_file_payload(&extra_file)
                }
            }]
        })),
    };

    let built = build_mrpack_from_base_archive_bytes(&approved, &base_archive, &[])
        .expect("mrpack should build from approved metadata");
    assert_eq!(
        built.manifest.get("status").and_then(|v| v.as_str()),
        Some("completed")
    );

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(built.archive_bytes)).unwrap();
    assert!(archive.by_name("overrides/config/base.toml").is_ok());
    let index: MrpackIndex = {
        let mut index_file = archive.by_name("modrinth.index.json").unwrap();
        serde_json::from_reader(&mut index_file).unwrap()
    };

    assert!(index.files.iter().any(|f| f.path == "mods/base.jar"));
    assert!(index.files.iter().any(|f| {
        f.path == "mods/extra.jar"
            && f.hashes.sha512 == "extra-sha512"
            && f.downloads == vec!["https://cdn.modrinth.com/data/extra/versions/v/extra.jar"]
    }));
}
