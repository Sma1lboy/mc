use super::base_search::{base_pack_provider_supported_for_execution, rank_base_packs};
use super::requirements::restriction_target_changed;
use super::*;

use crate::modpack::formats::mrpack::{
    EnvSupport, MrpackDependencies, MrpackEnv, MrpackFile, MrpackHashes, MrpackIndex,
};

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
            let mut buf = [0_u8; 16384];
            let _ = stream.read(&mut buf);
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
    let cfg = crate::agent::AgentLlmConfig::new("test-api-key");
    let llm = crate::agent::AgentLlmClient::new(cfg).unwrap();
    MainAgentRuntime::new(llm)
}

fn approval_route_runtime(decision: serde_json::Value) -> MainAgentRuntime {
    let body = openrouter_response_body(decision.to_string());
    let base_url = one_response_server(body);
    let mut cfg = crate::agent::AgentLlmConfig::new("test-api-key");
    cfg.base_url = base_url;
    let llm = crate::agent::AgentLlmClient::new(cfg).unwrap();
    MainAgentRuntime::new(llm)
}

fn openrouter_response_body(output_text: String) -> Vec<u8> {
    serde_json::json!({
        "id": "chatcmpl_test",
        "object": "chat.completion",
        "created": 0,
        "model": "gpt-test",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": output_text
            },
            "finish_reason": "stop",
            "native_finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2
        }
    })
    .to_string()
    .into_bytes()
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
        label: "Confirm recommended plan".to_string(),
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
        title: "Choose a base modpack".to_string(),
        message: "Choose one base pack".to_string(),
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
        available_decisions: approval_decisions("Choose this base pack", "Search base packs again"),
        tools: vec![update_build_restrictions_tool_spec()],
        plan: None,
    }
}

fn requirements_approval_run() -> AgentRunSnapshot {
    let restrictions = BuildRestrictions {
        revision: 1,
        minecraft_version: Some("1.20.1".to_string()),
        loader: Some("fabric".to_string()),
        feature_tags: vec!["performance".to_string()],
        ..Default::default()
    };
    let output = UpdateBuildRestrictionsOutput {
        restrictions: restrictions.clone(),
        missing_fields: Vec::new(),
        warnings: Vec::new(),
    };
    let mut run = AgentRunSnapshot::new("make a Fabric performance pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfigureRequirementsApproval;
    run.restrictions = Some(restrictions);
    run.pending_approval = Some(requirements_approval(&run.user_prompt, &output));
    run
}

fn base_pack_approval_run() -> AgentRunSnapshot {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ChooseBasePackApproval;
    run.plan = Some(test_plan());
    run.pending_approval = Some(test_approval());
    run
}

#[derive(Clone)]
struct WorkflowFakeProvider {
    versions: std::collections::HashMap<String, Vec<crate::modplatform::ProjectVersion>>,
}

impl WorkflowFakeProvider {
    fn new(
        versions: std::collections::HashMap<String, Vec<crate::modplatform::ProjectVersion>>,
    ) -> Self {
        Self { versions }
    }
}

impl crate::modplatform::provider::ResourceProvider for WorkflowFakeProvider {
    fn caps(&self) -> &crate::modplatform::ProviderCaps {
        static CAPS: crate::modplatform::ProviderCaps = crate::modplatform::ProviderCaps {
            id: ProviderId::Modrinth,
            readable_name: "Workflow Fake",
            hash_algos: &[],
            needs_api_key: false,
        };
        &CAPS
    }

    fn search<'a>(
        &'a self,
        _q: &'a SearchQuery,
    ) -> futures::future::BoxFuture<'a, Result<Vec<SearchHit>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn get_project<'a>(
        &'a self,
        project_id: &'a str,
    ) -> futures::future::BoxFuture<'a, Result<SearchHit>> {
        Box::pin(async move { Ok(test_search_hit(project_id, project_id)) })
    }

    fn get_projects<'a>(
        &'a self,
        project_ids: &'a [String],
    ) -> futures::future::BoxFuture<'a, Result<Vec<SearchHit>>> {
        Box::pin(async move {
            Ok(project_ids
                .iter()
                .map(|id| test_search_hit(id, id))
                .collect())
        })
    }

    fn list_versions<'a>(
        &'a self,
        project_id: &'a str,
        _game_version: Option<&'a str>,
        _loader: Option<&'a str>,
    ) -> futures::future::BoxFuture<'a, Result<Vec<crate::modplatform::ProjectVersion>>> {
        Box::pin(async move { Ok(self.versions.get(project_id).cloned().unwrap_or_default()) })
    }

    fn resolve_by_hashes<'a>(
        &'a self,
        _algo: crate::modplatform::HashAlgo,
        hashes: &'a [String],
    ) -> futures::future::BoxFuture<'a, Result<Vec<Option<ResolvedFile>>>> {
        Box::pin(async move { Ok(vec![None; hashes.len()]) })
    }

    fn get_files_bulk<'a>(
        &'a self,
        _refs: &'a [(String, String)],
    ) -> futures::future::BoxFuture<'a, Result<Vec<ResolvedFile>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn test_search_hit(id: &str, title: &str) -> SearchHit {
    SearchHit {
        id: id.to_string(),
        slug: id.to_string(),
        title: title.to_string(),
        description: format!("{title} description"),
        author: "test".to_string(),
        downloads: 100,
        icon_url: None,
        gallery_url: None,
        categories: Vec::new(),
        client_side: ProjectSideSupport::Required,
        server_side: ProjectSideSupport::Required,
    }
}

fn test_mod_candidate(id: &str, title: &str) -> ModCandidate {
    ModCandidate {
        provider: ProviderId::Modrinth,
        hit: test_search_hit(id, title),
        matched_query: title.to_string(),
    }
}

fn test_version_file(project_id: &str) -> VersionFile {
    VersionFile {
        url: format!("https://cdn.modrinth.com/data/{project_id}/versions/v/{project_id}.jar"),
        filename: format!("{project_id}.jar"),
        sha1: Some(format!("{project_id}-sha1")),
        sha512: Some(format!("{project_id}-sha512")),
        size: Some(123),
        primary: true,
        client_side: ProjectSideSupport::Required,
        server_side: ProjectSideSupport::Required,
    }
}

fn test_override_version_file(project_id: &str, filename: &str) -> VersionFile {
    let mut file = test_version_file(project_id);
    file.url = format!("https://example.com/download/{filename}");
    file.filename = filename.to_string();
    file
}

fn test_mrpack_file(path: &str, env: Option<(EnvSupport, EnvSupport)>) -> MrpackFile {
    let name = path
        .rsplit('/')
        .next()
        .unwrap_or("test.jar")
        .trim_end_matches(".jar");
    MrpackFile {
        path: path.to_string(),
        hashes: MrpackHashes {
            sha512: format!("{name}-sha512"),
            sha1: None,
        },
        env: env.map(|(client, server)| MrpackEnv { client, server }),
        downloads: vec![format!(
            "https://cdn.modrinth.com/data/{name}/versions/v/{name}.jar"
        )],
        file_size: Some(100),
    }
}

fn test_mrpack_index(files: Vec<MrpackFile>) -> MrpackIndex {
    MrpackIndex {
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
        files,
    }
}

fn test_extra_mod_ref(title: &str, project_id: &str, file: &VersionFile) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "project_id": project_id,
        "source_ref": {
            "kind": "mod_file",
            "provider": "modrinth",
            "project_id": project_id,
            "version_id": format!("{project_id}-version"),
            "file": version_file_payload(file)
        }
    })
}

fn test_approved_build(extra_mod_refs: Vec<serde_json::Value>) -> ApprovedModpackBuild {
    ApprovedModpackBuild {
        base_pack: serde_json::json!({
            "provider": "modrinth",
            "project_id": "base-project",
            "slug": "base-pack",
            "title": "Base Pack"
        }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
        extra_mods: Vec::new(),
        execution_recipe: Some(serde_json::json!({
            "schema_version": 1,
            "kind": "mrpack_from_base_modpack",
            "format": "mrpack",
            "extra_mod_refs": extra_mod_refs
        })),
    }
}

fn test_scratch_approved_build(
    loader: &str,
    extra_mod_refs: Vec<serde_json::Value>,
) -> ApprovedModpackBuild {
    ApprovedModpackBuild {
        base_pack: serde_json::json!({
            "provider": "scratch",
            "project_id": "scratch",
            "slug": "scratch",
            "title": "Start from scratch"
        }),
        target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": loader }),
        extra_mods: extra_mod_refs.clone(),
        execution_recipe: Some(serde_json::json!({
            "schema_version": 1,
            "kind": "mrpack_from_scratch",
            "format": "mrpack",
            "extra_mod_refs": extra_mod_refs
        })),
    }
}

fn test_project_version(
    project_id: &str,
    dependencies: Vec<crate::modplatform::Dependency>,
) -> crate::modplatform::ProjectVersion {
    crate::modplatform::ProjectVersion {
        id: format!("{project_id}-version"),
        name: format!("{project_id} Version"),
        version_number: "1.0.0".to_string(),
        game_versions: vec!["1.20.1".to_string()],
        loaders: vec!["fabric".to_string()],
        files: vec![test_version_file(project_id)],
        dependencies,
        client_side: ProjectSideSupport::Required,
        server_side: ProjectSideSupport::Required,
    }
}

fn test_dependency(project_id: &str, dependency_type: &str) -> crate::modplatform::Dependency {
    crate::modplatform::Dependency {
        project_id: Some(project_id.to_string()),
        version_id: None,
        dependency_type: dependency_type.to_string(),
    }
}

fn test_provider_registry(
    versions: Vec<(&str, Vec<crate::modplatform::Dependency>)>,
) -> ProviderRegistry {
    let versions = versions
        .into_iter()
        .map(|(project_id, dependencies)| {
            (
                project_id.to_string(),
                vec![test_project_version(project_id, dependencies)],
            )
        })
        .collect();
    ProviderRegistry::new().with(std::sync::Arc::new(WorkflowFakeProvider::new(versions)))
}

fn test_mod_plan_target() -> TargetCompatibility {
    TargetCompatibility {
        minecraft_version: Some("1.20.1".to_string()),
        loader: Some("fabric".to_string()),
        version_id: None,
        version_name: None,
        version_number: None,
        game_versions: vec!["1.20.1".to_string()],
        loaders: vec!["fabric".to_string()],
        primary_file: None,
        dependencies: Vec::new(),
    }
}

fn scratch_base_modlist() -> BaseModlistCache {
    BaseModlistCache {
        refs: Vec::new(),
        source_format: "scratch_empty".to_string(),
        fetch_count: 0,
    }
}

fn restrictions_with_tags(tags: &[&str]) -> BuildRestrictions {
    BuildRestrictions {
        feature_tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
        ..Default::default()
    }
}

fn initialized_mod_plan(tags: &[&str]) -> (TargetCompatibility, BaseModlistCache, ModPlanState) {
    let target = test_mod_plan_target();
    let base_modlist = scratch_base_modlist();
    let restrictions = restrictions_with_tags(tags);
    let state = initialize_mod_plan_state(&target, &base_modlist, Some(&restrictions));
    (target, base_modlist, state)
}

fn initialized_mod_plan_without_restrictions(
) -> (TargetCompatibility, BaseModlistCache, ModPlanState) {
    let target = test_mod_plan_target();
    let base_modlist = scratch_base_modlist();
    let state = initialize_mod_plan_state(&target, &base_modlist, None);
    (target, base_modlist, state)
}

fn first_theme_goal_id(state: &ModPlanState) -> String {
    state
        .goals
        .iter()
        .find(|goal| goal.kind == GoalKind::Theme)
        .map(|goal| goal.id.clone())
        .unwrap()
}

fn theme_goal_ids(state: &ModPlanState) -> Vec<String> {
    state
        .goals
        .iter()
        .filter(|goal| goal.kind == GoalKind::Theme)
        .map(|goal| goal.id.clone())
        .collect()
}

fn test_mod_selection(goal_id: impl Into<String>, project_id: &str) -> ModSelection {
    ModSelection {
        goal_id: goal_id.into(),
        project_id: project_id.to_string(),
    }
}

fn test_mod_plan_step(
    selections: Vec<ModSelection>,
    control: ModPlanControl,
    rationale: &str,
) -> ModPlanStep {
    ModPlanStep {
        selections,
        removals: Vec::new(),
        next_queries: Vec::new(),
        control,
        rationale: rationale.to_string(),
    }
}

fn test_selected_base_pack() -> SelectedBasePack {
    SelectedBasePack {
        provider: ProviderId::Modrinth,
        project_id: "base".to_string(),
        slug: "base".to_string(),
        title: "Base Pack".to_string(),
        description: Some("Base pack description".to_string()),
    }
}

fn customization_approval_run() -> AgentRunSnapshot {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.pending_approval = Some(ApprovalRequest {
        id: "approval-test".to_string(),
        kind: ApprovalKind::ConfirmCustomization,
        title: "Confirm customization plan".to_string(),
        message: "Ready to execute after confirmation".to_string(),
        options: vec![recommended_customization_option(1)],
        available_decisions: approval_decisions("Confirm recommended plan", "Change extra mods"),
        tools: Vec::new(),
        plan: Some(test_plan()),
    });
    run
}

fn approved_run_with_execution() -> AgentRunSnapshot {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.approved_build = Some(test_approved_build(Vec::new()));
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::Ready,
        manifest: Some(serde_json::json!({ "status": "ready" })),
        blocked: None,
    });
    run
}

fn execution_manifest_run(phase: AgentPhase) -> AgentRunSnapshot {
    let mut run = AgentRunSnapshot::new("make a pack");
    run.status = AgentStatus::Running;
    run.phase = phase;
    run.approved_build = Some(test_approved_build(Vec::new()));
    run
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
        label: "Confirm recommended plan".to_string(),
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
fn empty_base_pack_search_offers_scratch_fallback() {
    let approval = base_pack_selection_approval(&[], test_plan());

    assert_eq!(approval.kind, ApprovalKind::ChooseBasePack);
    assert!(approval
        .options
        .iter()
        .any(|option| option.id == "scratch:fallback"));
    assert!(approval
        .available_decisions
        .iter()
        .any(|d| d.kind == UserDecisionKind::Approve));
    assert!(approval.message.contains("Start from scratch"));
}

#[test]
fn legacy_scratch_fallback_choice_requires_model_reducer_path() {
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
    .expect("legacy scratch fallback should not hard fail");

    assert!(
        next.is_none(),
        "scratch selection must run through the async modplan reducer"
    );
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

    assert!(state
        .goals
        .iter()
        .any(|goal| goal.id == "baseline:fabric" && goal.status == GoalStatus::Covered));
    assert!(state
        .additions
        .iter()
        .any(|m| m.project_id == "P7dR8mSH" && m.provenance == ModProvenance::Baseline));
    assert!(state
        .pending_queries
        .iter()
        .any(|query| query.query == "ocean"));
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
        ModPlanControl::Done,
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
    assert!(state
        .goals
        .iter()
        .any(|goal| goal.id == goal_id && goal.status == GoalStatus::Covered));
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
        ModPlanControl::Continue,
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
    assert!(state
        .goals
        .iter()
        .any(|goal| goal.id == goal_id && goal.status == GoalStatus::Open));
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
        ModPlanControl::Continue,
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
    assert!(state
        .goals
        .iter()
        .any(|goal| goal.id == goals[0] && goal.status == GoalStatus::Covered));
    assert!(state
        .goals
        .iter()
        .any(|goal| goal.id == goals[1] && goal.status == GoalStatus::Open));
}

#[tokio::test]
async fn modplan_selection_can_restore_previous_removal() {
    let registry = test_provider_registry(vec![("root", Vec::new())]);
    let (_, _, mut state) = initialized_mod_plan(&["ocean"]);
    state.removals.push("root".to_string());
    let goal_id = first_theme_goal_id(&state);
    let step = test_mod_plan_step(
        vec![test_mod_selection(goal_id, "root")],
        ModPlanControl::Done,
        "restore root",
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
        ModPlanControl::Done,
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
    assert!(!blocked.details["unresolved_goals"]
        .as_array()
        .unwrap()
        .is_empty());
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
    assert!(state
        .pending_queries
        .iter()
        .any(|query| query.query == "more underwater survival"));
    assert!(state
        .pending_queries
        .iter()
        .any(|query| { query.query == "replace most Create addons with ocean exploration mods" }));
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
fn scratch_prompt_contract_matches_supported_fallback() {
    assert!(!MAIN_AGENT_SYSTEM_PROMPT.contains("Build-from-scratch is not available"));
    assert!(!SEARCH_QUERY_PROMPT.contains("Build-from-scratch is not available"));
    assert!(MAIN_AGENT_SYSTEM_PROMPT.contains("scratch"));
    assert!(SEARCH_QUERY_PROMPT.contains("scratch"));
}

#[test]
fn typed_approval_route_rejects_unknown_option() {
    let approval = test_approval();
    let text_err = parse_approval_decision_response(
            r#"{"decision":"approve","selected_option_id":"confirm:recommended_customization","message":null,"rationale":"wrong gate"}"#,
            &approval,
        )
        .expect_err("text approval route should reject options outside the pending gate");
    assert!(text_err.to_string().contains("unknown option"));

    let err = ApprovalRouteOutput {
        decision: ApprovalDecisionOutputKind::Approve,
        selected_option_id: Some("modrinth:missing".to_string()),
        message: None,
        rationale: "user chose an unavailable option".to_string(),
    }
    .into_route(&approval)
    .expect_err("typed approval route should reject unknown options");

    assert!(err.to_string().contains("unknown option"));
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
fn typed_search_queries_are_cleaned_deduped_and_truncated() {
    let queries = SearchQueryOutput {
        queries: vec![
            "1. Alpha".to_string(),
            "alpha".to_string(),
            "- Beta".to_string(),
            "queries".to_string(),
            "Gamma".to_string(),
            "Delta".to_string(),
            "Epsilon".to_string(),
        ],
    }
    .into_queries("base modpack search")
    .expect("typed search queries should normalize");

    assert_eq!(queries, vec!["Alpha", "Beta", "Gamma"]);
}

#[test]
fn typed_restriction_update_input_is_normalized_without_reparsing() {
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
fn typed_restriction_retry_accepts_valid_retry_without_reparsing() {
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
    .expect("typed retry value should validate without reparsing JSON text");

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

#[test]
fn typed_customization_critique_output_is_normalized_directly() {
    let critique = CustomizationCritiqueOutput {
        verdict: CustomizationCritiqueVerdictOutput::Revise,
        remove_project_ids: vec![" sodium ".to_string(), "".to_string(), "iris".to_string()],
        additional_queries: vec![
            "queries".to_string(),
            "1. Map Atlases".to_string(),
            "- Better Dungeons".to_string(),
            "Map Atlases".to_string(),
            "Extra Query".to_string(),
        ],
        rationale: "  needs more exploration  ".to_string(),
    }
    .into_critique()
    .expect("typed critique should normalize");

    assert_eq!(critique.verdict, CustomizationCritiqueVerdict::Revise);
    assert_eq!(critique.remove_project_ids, vec!["sodium", "iris"]);
    assert_eq!(
        critique.additional_queries,
        vec!["Map Atlases", "Better Dungeons"]
    );
    assert_eq!(critique.rationale, "needs more exploration");
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
fn search_queries_parse_and_clean_structured_output() {
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
        let queries = search_queries(input).expect("structured search-query output should parse");
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
        assert!(approval
            .available_decisions
            .iter()
            .any(|d| d.kind == UserDecisionKind::Approve));
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
    assert!(approved
        .execution_recipe
        .as_ref()
        .and_then(|v| v.get("extra_remote_files"))
        .is_none());
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

#[test]
fn final_customization_approval_can_continue_without_model() {
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
fn customization_back_can_continue_without_model() {
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

    let next = continue_modpack_build_without_model(run, decision)
        .expect("back action should be a deterministic state transition")
        .expect("back action should not require the model");

    assert_waiting_at(
        &next,
        AgentPhase::ChooseBasePackApproval,
        ApprovalKind::ChooseBasePack,
    );
    assert!(next.approved_build.is_none());
    assert!(next.execution.is_none());
    let approval = next
        .pending_approval
        .expect("base-pack approval should exist");
    assert!(approval
        .options
        .iter()
        .any(|option| option.id == "modrinth:base-project-1"));
    assert!(approval
        .available_decisions
        .iter()
        .any(|decision| decision.kind == UserDecisionKind::Revise));
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
    let run = execution_ready_run("https://example.com/base.mrpack", 10);
    let output = temp_mrpack_path("advance-verifying-failed");
    let runtime = test_main_runtime();

    let invalid_index = test_mrpack_index(vec![test_mrpack_file("mods/bad.jar", None)]);
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
        control: ModPlanControl::Continue,
        rationale: String::new(),
    };

    let normalized = step.normalized(&candidate_project_ids, &goal_ids);

    assert_eq!(normalized.next_queries.len(), 1);
    assert_eq!(normalized.next_queries[0].query, "Immersive Portals");
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
