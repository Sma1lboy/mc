use super::base_search::{
    base_pack_provider_supported_for_execution, block_base_pack_planning, rank_base_packs,
};
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

fn base_archive_for_artifact_tool() -> Vec<u8> {
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

fn initialized_mod_plan_without_restrictions()
-> (TargetCompatibility, BaseModlistCache, ModPlanState) {
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

fn test_mod_plan_step(selections: Vec<ModSelection>, rationale: &str) -> ModPlanStep {
    ModPlanStep {
        selections,
        removals: Vec::new(),
        next_queries: Vec::new(),
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

fn approval_draft(run: &AgentRunSnapshot, key: &str) -> ApprovalRequest {
    serde_json::from_value(
        run.agent_memory
            .get(key)
            .unwrap_or_else(|| panic!("missing approval draft: {key}"))
            .clone(),
    )
    .unwrap_or_else(|err| panic!("invalid approval draft {key}: {err}"))
}

fn requirements_output_draft(run: &AgentRunSnapshot) -> UpdateBuildRestrictionsOutput {
    serde_json::from_value(
        run.agent_memory
            .get("requirements_output")
            .expect("missing requirements output draft")
            .clone(),
    )
    .unwrap_or_else(|err| panic!("invalid requirements output draft: {err}"))
}

mod artifact_execution;
mod customization_flow;
mod execution_flow;
mod react_contract;
mod requirements_and_search;
