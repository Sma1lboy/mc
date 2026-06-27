use super::base_search::{base_pack_provider_supported_for_execution, rank_base_packs};
use super::*;

use crate::modpack::formats::mrpack::MrpackIndex;

fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    zip_bytes_with_dirs(&[], files)
}

fn zip_bytes_with_dirs(dirs: &[&str], files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::{Cursor, Write};
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        for dir in dirs {
            zip.add_directory(*dir, options).unwrap();
        }
        for (path, bytes) in files {
            zip.start_file(*path, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

fn one_response_server(body: Vec<u8>) -> String {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(headers.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    format!("http://{addr}")
}

fn temp_mrpack_path(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "mc-agent-{tag}-{}-{nanos}.mrpack",
        std::process::id()
    ))
}

fn test_main_runtime() -> MainAgentRuntime {
    let cfg = crate::agent::OpenAiConfig::new("test-api-key");
    let openai = crate::agent::OpenAiClient::new(cfg).unwrap();
    MainAgentRuntime::new(openai)
}

fn base_archive_for_advance() -> Vec<u8> {
    use crate::modpack::formats::mrpack::MrpackDependencies;

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
        files: Vec::new(),
    };
    let base_index_json = serde_json::to_vec(&base_index).unwrap();
    zip_bytes(&[("modrinth.index.json", &base_index_json)])
}

fn execution_ready_run(base_archive_url: &str, base_archive_size: u64) -> AgentRunSnapshot {
    let base_file = VersionFile {
        url: base_archive_url.to_string(),
        filename: "base.mrpack".to_string(),
        sha1: None,
        sha512: None,
        size: Some(base_archive_size),
        primary: true,
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    };
    let option = ApprovalOption {
        id: "confirm:recommended_customization".to_string(),
        label: "确认推荐方案".to_string(),
        description: None,
        payload: Some(serde_json::json!({
            "base_pack": {
                "provider": "modrinth",
                "project_id": "base-project",
                "slug": "base-pack",
                "title": "Base Pack"
            },
            "target": {
                "minecraft_version": "1.20.1",
                "loader": "fabric"
            },
            "extra_mods": [],
            "execution_recipe": {
                "schema_version": 1,
                "kind": "mrpack_from_base_modpack",
                "format": "mrpack",
                "base_pack_ref": {
                    "source_ref": {
                        "archive_file": version_file_payload(&base_file)
                    }
                },
                "extra_mod_refs": []
            }
        })),
    };
    continue_after_customization_confirmation(AgentRunSnapshot::new("make a pack"), option).unwrap()
}

fn test_approval() -> ApprovalRequest {
    ApprovalRequest {
        id: "approval-test".to_string(),
        kind: ApprovalKind::ChooseBasePack,
        title: "选择基础整合包".to_string(),
        message: "选择一个底包".to_string(),
        options: vec![
            ApprovalOption {
                id: "modrinth:first".to_string(),
                label: "First Pack".to_string(),
                description: None,
                payload: Some(serde_json::json!({ "provider": "modrinth" })),
            },
            ApprovalOption {
                id: "modrinth:second".to_string(),
                label: "Second Pack".to_string(),
                description: None,
                payload: Some(serde_json::json!({ "provider": "modrinth" })),
            },
        ],
        available_decisions: approval_decisions("选择该底包", "重新搜索底包"),
        tools: vec![update_build_restrictions_tool_spec()],
        plan: None,
    }
}

fn test_plan() -> ModpackAgentPlan {
    ModpackAgentPlan {
        objective: "make a pack".to_string(),
        summary_markdown: "test plan".to_string(),
        risks: Vec::new(),
        planned_actions: Vec::new(),
        migration_notes: Vec::new(),
    }
}

fn test_base_pack_payload(cycle: usize) -> serde_json::Value {
    serde_json::json!({
        "provider": "modrinth",
        "project_id": format!("base-project-{cycle}"),
        "slug": format!("base-pack-{cycle}"),
        "title": format!("Base Pack {cycle}"),
        "description": "A base pack used by workflow churn tests",
    })
}

fn recommended_customization_option(cycle: usize) -> ApprovalOption {
    ApprovalOption {
        id: "confirm:recommended_customization".to_string(),
        label: "确认推荐方案".to_string(),
        description: None,
        payload: Some(serde_json::json!({
            "base_pack": test_base_pack_payload(cycle),
            "target": {
                "minecraft_version": "1.20.1",
                "loader": "fabric"
            },
            "extra_mods": [{
                "provider": "modrinth",
                "project_id": format!("extra-project-{cycle}"),
                "slug": format!("extra-mod-{cycle}"),
                "title": format!("Extra Mod {cycle}")
            }],
            "execution_recipe": {
                "schema_version": 1,
                "kind": "mrpack_from_base_modpack",
                "format": "mrpack",
                "extra_mod_refs": []
            }
        })),
    }
}

fn assert_waiting_at(run: &AgentRunSnapshot, phase: AgentPhase, approval_kind: ApprovalKind) {
    assert_eq!(run.status, AgentStatus::WaitingForUser);
    assert_eq!(run.phase, phase);
    assert_eq!(
        run.pending_approval.as_ref().map(|a| &a.kind),
        Some(&approval_kind)
    );
}

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
fn repeated_workflow_reentry_keeps_state_machine_consistent() {
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
        assert_waiting_at(
            &run,
            AgentPhase::ConfigureRequirementsApproval,
            ApprovalKind::ConfigureRequirements,
        );
        assert!(run.approved_build.is_none());
        assert!(run.execution.is_none());
        assert_eq!(run.replans.len(), cycle);

        run = block_base_pack_planning(
            run,
            test_base_pack_payload(cycle),
            format!("cycle {cycle}: archive unavailable"),
        );
        assert_waiting_at(
            &run,
            AgentPhase::ChooseBasePackApproval,
            ApprovalKind::ChooseBasePack,
        );
        assert!(run
            .pending_approval
            .as_ref()
            .expect("base gate should be present")
            .options
            .iter()
            .all(|option| option.id != "scratch:fallback"));

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
        assert_waiting_at(
            &run,
            AgentPhase::ConfigureRequirementsApproval,
            ApprovalKind::ConfigureRequirements,
        );
        let missing_fields = run
            .pending_approval
            .as_ref()
            .and_then(|approval| approval.options.first())
            .and_then(|option| option.payload.as_ref())
            .and_then(|payload| payload.get("missing_fields"))
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
        assert_waiting_at(
            &run,
            AgentPhase::ConfirmCustomizationApproval,
            ApprovalKind::ConfirmCustomization,
        );
        assert!(run
            .pending_approval
            .as_ref()
            .expect("customization gate should be present")
            .options
            .iter()
            .all(|option| option.id != "confirm:recommended_customization"));

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
fn planning_entry_failure_blocks_at_base_pack_gate() {
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

    assert_eq!(next.status, AgentStatus::WaitingForUser);
    assert_eq!(next.phase, AgentPhase::ChooseBasePackApproval);
    assert!(next.approved_build.is_none());
    assert!(next.execution.is_none());
    assert_eq!(
        next.pending_approval.as_ref().map(|a| &a.kind),
        Some(&ApprovalKind::ChooseBasePack)
    );
}

#[test]
fn empty_base_pack_search_does_not_offer_scratch_fallback() {
    let approval = base_pack_selection_approval(&[], test_plan());

    assert_eq!(approval.kind, ApprovalKind::ChooseBasePack);
    assert!(approval.options.is_empty());
    assert!(!approval
        .available_decisions
        .iter()
        .any(|d| d.kind == UserDecisionKind::Approve));
    assert!(!approval.message.contains("fallback"));
}

#[test]
fn legacy_scratch_fallback_choice_recovers_to_base_pack_gate() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.pending_approval = Some(ApprovalRequest {
        id: "approval-test".to_string(),
        kind: ApprovalKind::ConfirmScratchFallback,
        title: "确认从零搭建".to_string(),
        message: "legacy scratch fallback".to_string(),
        options: vec![ApprovalOption {
            id: "confirm:scratch_fallback".to_string(),
            label: "确认从零搭建".to_string(),
            description: None,
            payload: Some(serde_json::json!({ "mode": "scratch_fallback" })),
        }],
        available_decisions: approval_decisions("确认从零搭建", "重新搜索底包"),
        tools: Vec::new(),
        plan: None,
    });

    let next = continue_modpack_build_without_model(
        run,
        UserDecision {
            approval_id: "approval-test".to_string(),
            kind: UserDecisionKind::Approve,
            selected_option_id: Some("confirm:scratch_fallback".to_string()),
            message: None,
            edits: serde_json::json!({}),
        },
    )
    .expect("legacy scratch fallback should not hard fail")
    .expect("legacy scratch fallback should recover");

    assert_eq!(next.status, AgentStatus::WaitingForUser);
    assert_eq!(next.phase, AgentPhase::ChooseBasePackApproval);
    let approval = next
        .pending_approval
        .expect("recovery gate should be present");
    assert_eq!(approval.kind, ApprovalKind::ChooseBasePack);
    assert!(approval.options.is_empty());
    assert!(!approval
        .available_decisions
        .iter()
        .any(|d| d.kind == UserDecisionKind::Approve));
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
    let approval = next
        .pending_approval
        .expect("blocked gate should be present");
    assert_eq!(approval.kind, ApprovalKind::ConfirmCustomization);
    assert!(approval
        .options
        .iter()
        .all(|o| o.id != "confirm:recommended_customization"));
    assert_eq!(next.phase, AgentPhase::ConfirmCustomizationApproval);
}

#[test]
fn customization_block_to_requirements_preserves_missing_fields() {
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

    assert_eq!(next.phase, AgentPhase::ConfigureRequirementsApproval);
    let missing = next
        .pending_approval
        .as_ref()
        .and_then(|a| a.options.first())
        .and_then(|o| o.payload.as_ref())
        .and_then(|p| p.get("missing_fields"))
        .and_then(|v| v.as_array())
        .expect("requirements approval should carry missing fields");
    assert_eq!(missing, &[serde_json::json!("minecraft_version")]);
}

#[test]
fn invalidate_downstream_is_idempotent() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.approved_build = Some(ApprovedModpackBuild {
        base_pack: serde_json::json!({ "title": "Base" }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: Vec::new(),
        execution_recipe: Some(serde_json::json!({ "format": "mrpack" })),
    });
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::Ready,
        manifest: Some(serde_json::json!({ "status": "ready" })),
        blocked: None,
    });

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
    let mut run = AgentRunSnapshot::new("make a pack");
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.approved_build = Some(ApprovedModpackBuild {
        base_pack: serde_json::json!({ "title": "Base" }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: vec![serde_json::json!({ "title": "Old Extra" })],
        execution_recipe: Some(serde_json::json!({ "format": "mrpack" })),
    });
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::Ready,
        manifest: Some(serde_json::json!({ "status": "ready" })),
        blocked: None,
    });

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
fn execution_retry_outcome_does_not_enter_failed() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::Running;
    run.phase = AgentPhase::Executing;

    let next = continue_after_execution_manifest_result(
        run,
        serde_json::json!({
            "status": "failed",
            "retryable": true,
            "error_kind": "source_timeout",
            "reason": "source timed out"
        }),
    )
    .expect("retryable external error should be accepted");

    assert_eq!(next.status, AgentStatus::Running);
    assert_eq!(next.phase, AgentPhase::Executing);
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::Retry)
    );
}

#[test]
fn failed_outcome_keeps_replan_gate_metadata() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::Running;
    run.phase = AgentPhase::Executing;

    let next = continue_after_execution_manifest_result(
        run,
        serde_json::json!({
            "status": "failed",
            "replan_phase": "base_pack",
            "reason": "corrupt archive"
        }),
    )
    .expect("failed manifest should be classified");

    assert_eq!(next.status, AgentStatus::Failed);
    assert_eq!(next.phase, AgentPhase::Failed);
    assert_eq!(
        next.execution
            .as_ref()
            .and_then(|e| e.blocked.as_ref())
            .and_then(|b| b.replan_phase.as_ref()),
        Some(&AgentPhase::ChooseBasePackApproval)
    );
}

#[test]
fn parses_llm_intent_json() {
    let intent = parse_intent_response(
        r#"{"intent":"build_modpack","confidence":0.91,"rationale":"user wants a pack"}"#,
    )
    .expect("intent json should parse");

    assert_eq!(intent.kind, AgentIntentKind::BuildModpack);
    assert!((intent.confidence - 0.91).abs() < 0.001);
    assert_eq!(intent.rationale.as_deref(), Some("user wants a pack"));
}

#[test]
fn parses_approval_decision_approve() {
    let approval = test_approval();
    let decision = parse_approval_decision_response(
            r#"{"decision":"approve","selected_option_id":"modrinth:second","message":null,"rationale":"user chose the second option"}"#,
            &approval,
        )
        .expect("approval decision should parse");

    assert_eq!(decision.approval_id, "approval-test");
    assert_eq!(decision.kind, UserDecisionKind::Approve);
    assert_eq!(
        decision.selected_option_id.as_deref(),
        Some("modrinth:second")
    );
}

#[test]
fn approval_decision_rejects_unknown_option() {
    let approval = test_approval();
    let err = parse_approval_decision_response(
            r#"{"decision":"approve","selected_option_id":"confirm:recommended_customization","message":null,"rationale":"wrong gate"}"#,
            &approval,
        )
        .expect_err("approval decision should reject options outside the pending gate");

    assert!(err.to_string().contains("unknown option"));
}

#[test]
fn parses_approval_decision_revise() {
    let approval = test_approval();
    let decision = parse_approval_decision_response(
            r#"{"decision":"revise","selected_option_id":null,"message":"换一批，更偏冒险探索","rationale":"user asked to search again"}"#,
            &approval,
        )
        .expect("revise decision should parse");

    assert_eq!(decision.kind, UserDecisionKind::Revise);
    assert_eq!(decision.message.as_deref(), Some("换一批，更偏冒险探索"));
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
fn unsupported_intent_maps_to_unknown() {
    let intent = parse_intent_response(r#"{"intent":"general_question","confidence":0.8}"#)
        .expect("intent json should parse");

    assert_eq!(intent.kind, AgentIntentKind::Unknown);
    assert!((intent.confidence - 0.8).abs() < 0.001);
}

#[test]
fn search_queries_parse_structured_output() {
    let queries = search_queries(r#"{"queries":["query one","query two","query three"]}"#)
        .expect("structured search-query output should parse");

    assert_eq!(queries, vec!["query one", "query two", "query three"]);
}

#[test]
fn search_queries_drop_model_header_values() {
    let queries = search_queries(
            r#"{"queries":["Create a base modpack search with these queries:","query one","query two"]}"#,
        )
        .expect("structured search-query output should parse");

    assert!(!queries
        .iter()
        .any(|q| q == "Create a base modpack search with these queries:"));
    assert_eq!(queries, vec!["query one", "query two"]);
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
fn rejects_repeated_restriction_tool_schema_instances() {
    let text = r#"{"base_revision":3,"patch":{"feature_tags":["adventure","exploration","qol"],"loader":"fabric","minecraft_version":"1.20.1","minecraft_version_requirement":"1.20.1","notes":"prefer dungeons and QoL"}}
{"base_revision":3,"patch":{"feature_tags":["adventure","exploration","qol"],"loader":"fabric","minecraft_version":"1.20.1","minecraft_version_requirement":"1.20.1","notes":"prefer dungeons and QoL"}}"#;

    let err = parse_restriction_update_response(text)
        .expect_err("schema parser should reject multiple root objects");

    assert!(err
        .to_string()
        .contains("single restriction tool schema object"));
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
        "改成偏探索",
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
fn complete_requirements_allow_approve_decision() {
    let input = parse_restriction_update_response(
            r#"{"base_revision":0,"patch":{"minecraft_version":"1.20.1","loader":"fabric","feature_tags":["industrial","automation"],"notes":null}}"#,
        )
        .expect("restriction tool json should parse");
    let output = update_build_restrictions(
        Some(BuildRestrictions::default()),
        input,
        BuildRestrictionChangeSource::InitialPrompt,
        "initial",
    )
    .expect("restriction update should apply");
    let approval = requirements_approval("make a Fabric 1.20.1 pack", &output);

    assert_eq!(approval.kind, ApprovalKind::ConfigureRequirements);
    assert!(approval
        .available_decisions
        .iter()
        .any(|d| d.kind == UserDecisionKind::Approve));
    assert_eq!(approval.tools[0].name, UPDATE_BUILD_RESTRICTIONS_TOOL);
    assert_eq!(approval.options[0].id, "requirements:detected");
}

#[test]
fn incomplete_requirements_still_offer_audit_approve_decision() {
    let input = parse_restriction_update_response(
            r#"{"base_revision":0,"patch":{"minecraft_version":null,"loader":null,"feature_tags":["adventure"],"notes":null}}"#,
        )
        .expect("restriction tool json should parse");
    let output = update_build_restrictions(
        Some(BuildRestrictions::default()),
        input,
        BuildRestrictionChangeSource::InitialPrompt,
        "initial",
    )
    .expect("restriction update should apply");
    let approval = requirements_approval("make an adventure pack", &output);

    assert!(approval
        .available_decisions
        .iter()
        .any(|d| d.kind == UserDecisionKind::Approve));
    let missing = approval.options[0]
        .payload
        .as_ref()
        .and_then(|p| p.get("missing_fields"))
        .and_then(|v| v.as_array())
        .expect("missing fields should be present");
    assert_eq!(missing.len(), 2);
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
        summary.contains("缺少: Minecraft version"),
        "unexpected summary: {summary}"
    );
}

#[test]
fn requested_compatibility_comes_from_typed_restrictions() {
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
        "改成 Forge 1.19.2，更偏冒险探索",
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

    assert_eq!(next.status, AgentStatus::WaitingForUser);
    assert_eq!(next.phase, AgentPhase::ConfigureRequirementsApproval);
    assert!(next.approved_build.is_none());
    assert!(next.execution.is_none());
    assert_eq!(
        next.pending_approval.as_ref().map(|a| &a.kind),
        Some(&ApprovalKind::ConfigureRequirements)
    );
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
    let hit = SearchHit {
        id: "project".to_string(),
        slug: "project-slug".to_string(),
        title: "Project Title".to_string(),
        description: "Short project description".to_string(),
        author: "author".to_string(),
        downloads: 42,
        icon_url: None,
        gallery_url: None,
        categories: vec!["tech".to_string()],
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    };

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

    assert!(base_option
        .payload
        .as_ref()
        .and_then(|p| p.get("describe"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| s.contains("tech adventure")));
    assert!(mod_value
        .get("describe")
        .and_then(|v| v.as_str())
        .is_some_and(|s| s.contains("backpack")));
}

#[test]
fn resolved_mod_payload_contains_source_metadata_not_execution_manifest() {
    let hit = SearchHit {
        id: "mod-project".to_string(),
        slug: "mod-slug".to_string(),
        title: "Mod Title".to_string(),
        description: "Adds exploration".to_string(),
        author: "author".to_string(),
        downloads: 99,
        icon_url: None,
        gallery_url: None,
        categories: vec!["adventure".to_string()],
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    };
    let file = VersionFile {
        url: "https://cdn.modrinth.com/data/mod-project/versions/v/mod.jar".to_string(),
        filename: "mod.jar".to_string(),
        sha1: Some("sha1".to_string()),
        sha512: Some("sha512".to_string()),
        size: Some(1234),
        primary: true,
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    };
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
    let file = VersionFile {
        url: "https://cdn.modrinth.com/data/mod-project/versions/v/mod.jar".to_string(),
        filename: "../nested/evil.jar".to_string(),
        sha1: Some("sha1".to_string()),
        sha512: Some("sha512".to_string()),
        size: Some(1234),
        primary: true,
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    };

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
    let file = VersionFile {
        url: "https://cdn.modrinth.com/data/mod-project/versions/v/mod.jar".to_string(),
        filename: "mod.jar".to_string(),
        sha1: Some("sha1".to_string()),
        sha512: Some("sha512".to_string()),
        size: Some(123),
        primary: true,
        client_side: ProjectSideSupport::Required,
        server_side: ProjectSideSupport::Unsupported,
    };

    let payload = mrpack_file_payload(&file).expect("remote payload should compile");

    assert_eq!(
        payload
            .get("env")
            .and_then(|v| v.get("client"))
            .and_then(|v| v.as_str()),
        Some("required")
    );
    assert_eq!(
        payload
            .get("env")
            .and_then(|v| v.get("server"))
            .and_then(|v| v.as_str()),
        Some("unsupported")
    );
}

#[test]
fn mrpack_file_payload_falls_back_unknown_env_to_optional() {
    let file = VersionFile {
        url: "https://cdn.modrinth.com/data/mod-project/versions/v/mod.jar".to_string(),
        filename: "mod.jar".to_string(),
        sha1: Some("sha1".to_string()),
        sha512: Some("sha512".to_string()),
        size: Some(123),
        primary: true,
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    };

    let payload = mrpack_file_payload(&file).expect("remote payload should compile");

    assert_eq!(
        payload
            .get("env")
            .and_then(|v| v.get("client"))
            .and_then(|v| v.as_str()),
        Some("optional")
    );
    assert_eq!(
        payload
            .get("env")
            .and_then(|v| v.get("server"))
            .and_then(|v| v.as_str()),
        Some("optional")
    );
}

#[test]
fn exec_compile_metadata_merges_base_index_and_extra_mod_refs() {
    use crate::modpack::formats::mrpack::{
        EnvSupport, MrpackDependencies, MrpackEnv, MrpackFile, MrpackHashes,
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
            env: Some(MrpackEnv {
                client: EnvSupport::Required,
                server: EnvSupport::Required,
            }),
            downloads: vec!["https://cdn.modrinth.com/data/base/base.jar".to_string()],
            file_size: Some(100),
        }],
    };
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
    assert!(approved
        .execution_recipe
        .as_ref()
        .and_then(|v| v.get("extra_remote_files"))
        .is_none());
}

#[test]
fn exec_compile_metadata_applies_base_file_env_overrides() {
    use std::collections::HashMap;

    use crate::modpack::formats::mrpack::{
        EnvSupport, MrpackDependencies, MrpackEnv, MrpackFile, MrpackHashes,
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
        files: vec![
            MrpackFile {
                path: "mods/client.jar".to_string(),
                hashes: MrpackHashes {
                    sha512: "client-sha512".to_string(),
                    sha1: None,
                },
                env: None,
                downloads: vec![
                    "https://cdn.modrinth.com/data/client-project/versions/v/client.jar"
                        .to_string(),
                ],
                file_size: Some(100),
            },
            MrpackFile {
                path: "mods/fallback.jar".to_string(),
                hashes: MrpackHashes {
                    sha512: "fallback-sha512".to_string(),
                    sha1: None,
                },
                env: None,
                downloads: vec![
                    "https://cdn.modrinth.com/data/fallback-project/versions/v/fallback.jar"
                        .to_string(),
                ],
                file_size: Some(100),
            },
            MrpackFile {
                path: "mods/explicit.jar".to_string(),
                hashes: MrpackHashes {
                    sha512: "explicit-sha512".to_string(),
                    sha1: None,
                },
                env: Some(MrpackEnv {
                    client: EnvSupport::Optional,
                    server: EnvSupport::Unsupported,
                }),
                downloads: vec![
                    "https://cdn.modrinth.com/data/explicit-project/versions/v/explicit.jar"
                        .to_string(),
                ],
                file_size: Some(100),
            },
        ],
    };
    let approved = ApprovedModpackBuild {
        base_pack: serde_json::json!({ "title": "Base Pack" }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: Vec::new(),
        execution_recipe: Some(serde_json::json!({
            "schema_version": 1,
            "kind": "mrpack_from_base_modpack",
            "format": "mrpack",
            "extra_mod_refs": []
        })),
    };
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
    use crate::modpack::formats::mrpack::{MrpackDependencies, MrpackIndex};

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
        files: Vec::new(),
    };
    let extra_file = VersionFile {
        url: "https://example.com/download/evil.jar".to_string(),
        filename: "..\\nested\\evil.jar".to_string(),
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
fn exec_compile_metadata_blocks_unverifiable_override_source() {
    use crate::modpack::formats::mrpack::{MrpackDependencies, MrpackIndex};

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
        files: Vec::new(),
    };
    let extra_file = VersionFile {
        url: "https://example.com/download/extra.jar".to_string(),
        filename: "extra.jar".to_string(),
        sha1: None,
        sha512: None,
        size: None,
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
                "title": "Unverified Override",
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
fn execution_manifest_ready_enters_executing_phase() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::Running;
    run.phase = AgentPhase::ExecutionReady;
    run.approved_build = Some(ApprovedModpackBuild {
        base_pack: serde_json::json!({ "title": "Base Pack" }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: Vec::new(),
        execution_recipe: Some(serde_json::json!({
            "kind": "mrpack_from_base_modpack",
            "format": "mrpack"
        })),
    });

    let next = continue_after_execution_manifest_result(
        run,
        serde_json::json!({
            "status": "ready",
            "format": "mrpack",
            "output_index": { "files": [] }
        }),
    )
    .expect("ready manifest should advance");

    assert_eq!(next.status, AgentStatus::Running);
    assert_eq!(next.phase, AgentPhase::Executing);
    assert!(next.pending_approval.is_none());
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::Ready)
    );
}

#[test]
fn execution_completed_outcome_enters_completed_phase() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::Running;
    run.phase = AgentPhase::Executing;

    let next = continue_after_execution_manifest_result(
        run,
        serde_json::json!({
            "status": "completed",
            "output_path": "/tmp/pack.mrpack"
        }),
    )
    .expect("completed execution manifest should complete the run");

    assert_eq!(next.status, AgentStatus::Completed);
    assert_eq!(next.phase, AgentPhase::Completed);
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::Completed)
    );
}

#[test]
fn execution_manifest_blocked_returns_to_customization_gate() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::Running;
    run.phase = AgentPhase::ExecutionReady;
    run.approved_build = Some(ApprovedModpackBuild {
        base_pack: serde_json::json!({
            "provider": "modrinth",
            "project_id": "base-project",
            "slug": "base-pack",
            "title": "Base Pack"
        }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: vec![serde_json::json!({
            "provider": "modrinth",
            "project_id": "extra-project",
            "slug": "extra-mod",
            "title": "Extra Mod",
            "source_ref": {
                "kind": "mod_file",
                "provider": "modrinth",
                "project_id": "extra-project",
                "version_id": "extra-version"
            }
        })],
        execution_recipe: Some(serde_json::json!({
            "kind": "mrpack_from_base_modpack",
            "format": "mrpack",
            "extra_mod_refs": []
        })),
    });

    let next = continue_after_execution_manifest_result(
        run,
        serde_json::json!({
            "status": "blocked",
            "replan_phase": "confirm_customization_approval",
            "blocked": [{
                "title": "Extra Mod",
                "reason": "missing resolved source file"
            }]
        }),
    )
    .expect("blocked manifest should return to a HITL gate");

    assert_eq!(next.status, AgentStatus::WaitingForUser);
    assert_eq!(next.phase, AgentPhase::ConfirmCustomizationApproval);
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::Blocked)
    );
    let approval = next.pending_approval.expect("approval should be restored");
    assert_eq!(approval.kind, ApprovalKind::ConfirmCustomization);
    assert!(approval.message.contains("missing resolved source file"));
}

#[test]
fn execution_manifest_blocked_returns_to_base_pack_gate() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::Running;
    run.phase = AgentPhase::ExecutionReady;
    run.approved_build = Some(ApprovedModpackBuild {
        base_pack: serde_json::json!({
            "provider": "modrinth",
            "project_id": "base-project",
            "slug": "base-pack",
            "title": "Base Pack"
        }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: Vec::new(),
        execution_recipe: Some(serde_json::json!({
            "kind": "mrpack_from_base_modpack",
            "format": "mrpack"
        })),
    });

    let next = continue_after_execution_manifest_result(
        run,
        serde_json::json!({
            "status": "blocked",
            "replan_phase": "base_pack",
            "blocked": [{
                "title": "Base Pack",
                "reason": "base archive missing modrinth.index.json"
            }]
        }),
    )
    .expect("base-pack execution block should return to base-pack HITL");

    assert_eq!(next.status, AgentStatus::WaitingForUser);
    assert_eq!(next.phase, AgentPhase::ChooseBasePackApproval);
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::Blocked)
    );
    let approval = next.pending_approval.expect("approval should be restored");
    assert_eq!(approval.kind, ApprovalKind::ChooseBasePack);
    assert!(approval
        .message
        .contains("base archive missing modrinth.index.json"));
}

#[test]
fn execution_manifest_blocked_returns_to_requirements_gate() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::Running;
    run.phase = AgentPhase::ExecutionReady;
    run.restrictions = Some(BuildRestrictions {
        minecraft_version: Some("1.20.1".to_string()),
        loader: Some("fabric".to_string()),
        ..BuildRestrictions::default()
    });

    let next = continue_after_execution_manifest_result(
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
    .expect("requirements execution block should return to requirements HITL");

    assert_eq!(next.status, AgentStatus::WaitingForUser);
    assert_eq!(next.phase, AgentPhase::ConfigureRequirementsApproval);
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::Blocked)
    );
    let approval = next.pending_approval.expect("approval should be restored");
    assert_eq!(approval.kind, ApprovalKind::ConfigureRequirements);
    assert!(approval
        .message
        .contains("selected loader is incompatible with requested version"));
}

#[test]
fn final_customization_approval_can_continue_without_model() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.pending_approval = Some(ApprovalRequest {
        id: "approval-test".to_string(),
        kind: ApprovalKind::ConfirmCustomization,
        title: "确认定制方案".to_string(),
        message: "确认后执行".to_string(),
        options: vec![ApprovalOption {
            id: "confirm:recommended_customization".to_string(),
            label: "确认推荐方案".to_string(),
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
        available_decisions: approval_decisions("确认推荐方案", "修改补充 mods"),
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

    let next = continue_modpack_build_without_model(run, decision)
        .expect("offline continuation should not need provider")
        .expect("final approval should complete offline");

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
fn blocked_customization_gate_does_not_advertise_unusable_revise() {
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
    let approval = next
        .pending_approval
        .expect("blocked approval should be present");

    assert!(
        !approval
            .available_decisions
            .iter()
            .any(|d| d.kind == UserDecisionKind::Revise),
        "blocked customization gate must not advertise a revise path without a recommended customization payload"
    );
}

#[tokio::test]
async fn advance_executes_approved_execution_ready_run_to_completed() {
    let base_archive = base_archive_for_advance();
    let base_url = one_response_server(base_archive.clone());
    let run = execution_ready_run(
        &format!("{base_url}/base.mrpack"),
        base_archive.len() as u64,
    );
    let tool = run
        .tools
        .iter()
        .find(|tool| tool.name == "build_mrpack_artifact")
        .expect("execution-ready run should expose deterministic execution tool");
    assert_eq!(
        tool.input_schema
            .get("properties")
            .and_then(|v| v.get("output_path"))
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str()),
        Some("string")
    );
    let output = temp_mrpack_path("advance-completed");
    let runtime = test_main_runtime();

    let next = runtime
        .advance(run, &output)
        .await
        .expect("advance should drive deterministic execution");

    assert_eq!(next.status, AgentStatus::Completed);
    assert_eq!(next.phase, AgentPhase::Completed);
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::Completed)
    );
    assert!(next
        .trace
        .iter()
        .any(|event| event.event.contains("entering verifying phase")));
    assert!(next
        .trace
        .iter()
        .any(|event| event.event.contains("verification completed")));
    assert!(output.exists(), "advance should write the mrpack artifact");
    let _ = std::fs::remove_file(output);
}

#[tokio::test]
async fn advance_fails_during_verifying_for_invalid_artifact() {
    use crate::modpack::formats::mrpack::{
        MrpackDependencies, MrpackFile, MrpackHashes, MrpackIndex,
    };

    let run = execution_ready_run("https://example.com/base.mrpack", 10);
    let output = temp_mrpack_path("advance-verifying-failed");
    let runtime = test_main_runtime();

    let invalid_index = MrpackIndex {
        format_version: 1,
        game: "minecraft".to_string(),
        version_id: "bad-1.0.0".to_string(),
        name: "Bad Pack".to_string(),
        summary: None,
        dependencies: MrpackDependencies {
            minecraft: Some("1.20.1".to_string()),
            fabric_loader: Some("0.15.7".to_string()),
            ..Default::default()
        },
        files: vec![MrpackFile {
            path: "mods/bad.jar".to_string(),
            hashes: MrpackHashes {
                sha512: "sha512".to_string(),
                sha1: None,
            },
            env: None,
            downloads: vec!["https://cdn.modrinth.com/data/bad/bad.jar".to_string()],
            file_size: Some(10),
        }],
    };
    let invalid_index_json = serde_json::to_vec(&invalid_index).unwrap();
    let archive = zip_bytes(&[("modrinth.index.json", &invalid_index_json)]);

    let next = runtime
        .advance_with_executor(
            run,
            &output,
            move |_approved, path| {
                let archive = archive.clone();
                async move {
                    std::fs::write(&path, archive).unwrap();
                    Ok(serde_json::json!({
                        "schema_version": 1,
                        "status": "verifying",
                        "format": "mrpack",
                        "output_path": path.to_string_lossy().to_string()
                    }))
                }
            },
            std::time::Duration::ZERO,
        )
        .await
        .expect("verification failures should be represented in run state");

    assert_eq!(next.status, AgentStatus::Failed);
    assert_eq!(next.phase, AgentPhase::Failed);
    assert_eq!(
        next.execution.as_ref().map(|e| &e.status),
        Some(&AgentExecutionStatus::Failed)
    );
    assert!(next
        .execution
        .as_ref()
        .and_then(|e| e.blocked.as_ref())
        .map(|blocked| blocked.reason.contains("missing env"))
        .unwrap_or(false));
    assert!(next
        .trace
        .iter()
        .any(|event| event.event.contains("entering verifying phase")));
    let _ = std::fs::remove_file(output);
}

#[test]
fn verify_written_mrpack_rejects_deep_invalid_artifacts() {
    use crate::modpack::formats::mrpack::{
        EnvSupport, MrpackDependencies, MrpackEnv, MrpackFile, MrpackHashes, MrpackIndex,
    };

    let approved = ApprovedModpackBuild {
        base_pack: serde_json::json!({ "title": "Base Pack" }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: Vec::new(),
        execution_recipe: None,
    };
    let valid_file = MrpackFile {
        path: "mods/valid.jar".to_string(),
        hashes: MrpackHashes {
            sha512: "sha512".to_string(),
            sha1: None,
        },
        env: Some(MrpackEnv {
            client: EnvSupport::Required,
            server: EnvSupport::Unsupported,
        }),
        downloads: vec!["https://cdn.modrinth.com/data/valid/valid.jar".to_string()],
        file_size: Some(10),
    };
    let valid_index = MrpackIndex {
        format_version: 1,
        game: "minecraft".to_string(),
        version_id: "verify-1.0.0".to_string(),
        name: "Verify Pack".to_string(),
        summary: None,
        dependencies: MrpackDependencies {
            minecraft: Some("1.20.1".to_string()),
            fabric_loader: Some("0.15.7".to_string()),
            ..Default::default()
        },
        files: vec![valid_file.clone()],
    };

    let mut no_downloads = valid_index.clone();
    no_downloads.files[0].downloads.clear();
    let mut no_env = valid_index.clone();
    no_env.files[0].env = None;

    let cases = vec![
        ("no-downloads", no_downloads, Vec::new(), "missing downloads"),
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

#[tokio::test]
async fn advance_caps_retry_manifests_without_hot_loop() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let base_archive = base_archive_for_advance();
    let run = execution_ready_run("https://example.com/base.mrpack", base_archive.len() as u64);
    let output = temp_mrpack_path("advance-retry-cap");
    let runtime = test_main_runtime();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_executor = Arc::clone(&attempts);

    let next = runtime
        .advance_with_executor(
            run,
            &output,
            move |_approved, _path| {
                let attempts_for_executor = Arc::clone(&attempts_for_executor);
                async move {
                    attempts_for_executor.fetch_add(1, Ordering::SeqCst);
                    Ok(serde_json::json!({
                        "schema_version": 1,
                        "status": "retry",
                        "format": "mrpack",
                        "reason": "source timeout",
                        "error_kind": "network_timeout",
                        "retryable": true
                    }))
                }
            },
            std::time::Duration::ZERO,
        )
        .await
        .expect("retry cap should return a terminal run");

    assert_eq!(
        attempts.load(Ordering::SeqCst),
        3,
        "retry driver should stop at the configured cap"
    );
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
    assert!(!output.exists(), "retry exhaustion must not write output");
}

#[tokio::test]
async fn advance_stops_at_waiting_for_user_gate_without_executing() {
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
    let output = temp_mrpack_path("advance-waiting");
    let runtime = test_main_runtime();

    let next = runtime
        .advance(run.clone(), &output)
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
        "advance must not execute past a HITL gate"
    );
}

#[tokio::test]
async fn advance_does_not_reexecute_completed_run() {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::Completed;
    run.phase = AgentPhase::Completed;
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::Completed,
        manifest: Some(serde_json::json!({ "status": "completed" })),
        blocked: None,
    });
    let trace_len = run.trace.len();
    let output = temp_mrpack_path("advance-idempotent");
    let runtime = test_main_runtime();

    let next = runtime
        .advance(run, &output)
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
    assert!(manifest
        .get("reason")
        .and_then(|v| v.as_str())
        .is_some_and(|reason| reason.contains("provider is not supported")));
    assert!(!output.exists());
}

#[test]
fn search_query_cleanup_preserves_digit_prefixed_project_names() {
    let value =
        search_queries(r#"{"queries":["3D Skin Layers","1. Better Dungeons","- Map Atlases"]}"#)
            .expect("queries should parse");

    assert_eq!(
        value,
        vec![
            "3D Skin Layers".to_string(),
            "Better Dungeons".to_string(),
            "Map Atlases".to_string()
        ]
    );
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
