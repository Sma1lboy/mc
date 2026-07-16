use super::*;
use crate::agent_history::AgentHistoryStore;

const MAX_CLOUD_HISTORY_RECORD_BYTES: usize = 1_048_576;

/// A published agent chat transcript: its short id + the public fetch URL.
#[derive(serde::Serialize, specta::Type)]
pub struct SharedConversation {
    pub id: String,
    pub url: String,
}

fn public_share_payload(payload: &serde_json::Value) -> CmdResult<serde_json::Value> {
    mc_core::agent::conversation_privacy::project_public_share(payload).map_err(err)
}

/// Publish the current agent chat transcript to the deployed mc-server for public
/// sharing (always cloud — no local fallback). Requires a signed-in kobeMC
/// account: uses the shared managed client so the better-auth session cookie is
/// sent. `payload_json` is the JSON.stringify'd transcript (String, to avoid
/// exporting a recursive `serde_json::Value` through specta).
#[tauri::command]
#[specta::specta]
pub async fn agent_share_conversation(client: tauri::State<'_, mc_core::server::ServerClient>, payload_json: String) -> CmdResult<SharedConversation> {
    let raw: serde_json::Value = serde_json::from_str(&payload_json).map_err(err)?;
    let payload = public_share_payload(&raw)?;
    let (id, url) = client.share_conversation(&payload).await.map_err(err)?;
    Ok(SharedConversation { id, url })
}

// --- agent conversation history (cloud sync; authed) -------------------------
// Thin wrappers over `ServerClient::agent_history_*`. Records travel as JSON
// strings (same reason as `agent_share_conversation`: no recursive Value in specta).

fn agent_local_history_store() -> CmdResult<AgentHistoryStore> {
    let store = AgentHistoryStore::open(data_dir().join("agent").join("history.sqlite3"))?;
    store.import_legacy_webkit_once()?;
    Ok(store)
}

pub fn initialize_agent_local_history() {
    if let Err(error) = agent_local_history_store() {
        tracing::warn!(target: "agent_history", "初始化本地聊天历史失败: {error}");
    }
}

async fn load_agent_history_records() -> CmdResult<Vec<String>> {
    tokio::task::spawn_blocking(|| agent_local_history_store()?.load_all()).await.map_err(err)?
}

async fn save_agent_history_record(id: String, record_json: String) -> CmdResult<()> {
    tokio::task::spawn_blocking(move || agent_local_history_store()?.upsert(&id, &record_json)).await.map_err(err)?
}

async fn stored_agent_history_owner(id: &str) -> CmdResult<Option<Option<String>>> {
    let records = load_agent_history_records().await?;
    Ok(records.into_iter().find_map(|raw| {
        let (record_id, _, record) = agent_history_metadata(&raw)?;
        (record_id == id).then(|| {
            mc_core::agent::conversation_privacy::conversation_record_owner(&record)
                .map(str::to_owned)
        })
    }))
}

fn owner_after_session_error(
    error: &mc_core::error::CoreError,
    stored_owner: Option<Option<String>>,
) -> CmdResult<Option<String>> {
    if let Some(stored_owner) = stored_owner {
        return Ok(stored_owner);
    }
    if matches!(error, mc_core::error::CoreError::Auth(_)) {
        return Ok(None);
    }
    Err(format!("cannot establish conversation owner: {error}"))
}

fn agent_history_metadata(raw: &str) -> Option<(String, i64, serde_json::Value)> {
    let record: serde_json::Value = serde_json::from_str(raw).ok()?;
    let id = record.get("id")?.as_str()?.to_string();
    if id.is_empty() || !record.get("messages")?.is_array() {
        return None;
    }
    let updated_at_ms = record.get("updatedAt").and_then(serde_json::Value::as_i64).unwrap_or_default();
    Some((id, updated_at_ms, record))
}

fn agent_history_owned_by(record: &serde_json::Value, owner_id: &str) -> bool {
    mc_core::agent::conversation_privacy::conversation_record_owner(record) == Some(owner_id)
}

fn agent_history_visible_to(record: &serde_json::Value, owner_id: Option<&str>) -> bool {
    mc_core::agent::conversation_privacy::conversation_record_visible_to(record, owner_id)
}

async fn visible_agent_history(owner_id: Option<&str>) -> CmdResult<Vec<String>> {
    let records = load_agent_history_records().await?;
    let mut visible = Vec::new();
    for raw in records {
        let Some((id, _, record)) = agent_history_metadata(&raw) else {
            continue;
        };
        if !agent_history_visible_to(&record, owner_id) {
            continue;
        }
        let projected_owner = mc_core::agent::conversation_privacy::conversation_record_owner(&record);
        let projected = mc_core::agent::conversation_privacy::project_conversation_record(&record, projected_owner).map_err(err)?;
        let projected_raw = serde_json::to_string(&projected).map_err(err)?;
        if projected_raw != raw {
            save_agent_history_record(id, projected_raw.clone()).await?;
        }
        visible.push(projected_raw);
    }
    Ok(visible)
}

#[tauri::command]
#[specta::specta]
pub async fn agent_history_hydrate(client: tauri::State<'_, mc_core::server::ServerClient>) -> CmdResult<String> {
    let owner_id = client.me().await.ok().map(|user| user.id);
    serde_json::to_string(&visible_agent_history(owner_id.as_deref()).await?).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn agent_history_sync(client: tauri::State<'_, mc_core::server::ServerClient>, owner_id: String) -> CmdResult<String> {
    let _ = owner_id;
    let owner_id = client.me().await.map_err(err)?.id;
    let local = load_agent_history_records().await?;
    let remote_heads = client.agent_history_list().await.map_err(err)?;
    let local_updated: HashMap<String, i64> = local
        .iter()
        .filter_map(|raw| {
            let (id, updated, record) = agent_history_metadata(raw)?;
            agent_history_owned_by(&record, &owner_id).then_some((id, updated))
        })
        .collect();
    for head in &remote_heads {
        if local_updated.get(&head.id).is_some_and(|updated| *updated >= head.updated_at_ms) {
            continue;
        }
        let Ok(record) = client.agent_history_get(&head.id).await else {
            continue;
        };
        let Ok(projected) = mc_core::agent::conversation_privacy::project_conversation_record(&record, Some(&owner_id)) else {
            continue;
        };
        let raw = serde_json::to_string(&projected).map_err(err)?;
        if let Some((id, _, _)) = agent_history_metadata(&raw) {
            save_agent_history_record(id, raw).await?;
        }
    }
    let merged = load_agent_history_records().await?;
    let remote_updated: HashMap<&str, i64> = remote_heads.iter().map(|head| (head.id.as_str(), head.updated_at_ms)).collect();
    for raw in merged {
        let Some((id, updated, record)) = agent_history_metadata(&raw) else {
            continue;
        };
        if !agent_history_owned_by(&record, &owner_id) {
            continue;
        }
        let projected = mc_core::agent::conversation_privacy::project_conversation_record(&record, Some(&owner_id)).map_err(err)?;
        let projected_raw = serde_json::to_string(&projected).map_err(err)?;
        if projected_raw != raw {
            save_agent_history_record(id.clone(), projected_raw.clone()).await?;
        }
        if projected_raw.len() <= MAX_CLOUD_HISTORY_RECORD_BYTES
            && remote_updated
                .get(id.as_str())
                .is_none_or(|remote| updated > *remote)
        {
            let _ = client.agent_history_put(&id, &projected).await;
        }
    }
    serde_json::to_string(&visible_agent_history(Some(&owner_id)).await?).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn agent_history_save(client: tauri::State<'_, mc_core::server::ServerClient>, id: String, record_json: String, current_owner_id: Option<String>) -> CmdResult<()> {
    let _ = current_owner_id;
    let record: serde_json::Value = serde_json::from_str(&record_json).map_err(err)?;
    let authenticated_owner = match client.me().await {
        Ok(user) => Some(user.id),
        Err(error) => owner_after_session_error(
            &error,
            stored_agent_history_owner(&id).await?,
        )?,
    };
    let projected = mc_core::agent::conversation_privacy::project_conversation_record(&record, authenticated_owner.as_deref()).map_err(err)?;
    if projected.get("id").and_then(serde_json::Value::as_str) != Some(id.as_str()) {
        return Err("conversation record id does not match storage key".into());
    }
    let projected_raw = serde_json::to_string(&projected).map_err(err)?;
    save_agent_history_record(id.clone(), projected_raw.clone()).await?;
    if authenticated_owner.is_some() && projected_raw.len() <= MAX_CLOUD_HISTORY_RECORD_BYTES {
        let _ = client.agent_history_put(&id, &projected).await;
    }
    Ok(())
}

// --- agent deterministic tools (for a TS-side agent loop) -----------------
//
// Six deterministic modpack tools, exposed one-per-command so the TS agent brain
// (Vercel AI SDK in the webview) can run the tool-use loop itself and dispatch each
// tool via `invoke()`. Every command is a thin wrapper over the single-source
// `tool_*` fn in `mc_core::agent::tools` — no logic
// here. Safety is unchanged: the tools only ever return real provider/resolver
// data, and `agent_tool_build_modpack` re-resolves every file through the provider.

/// Shared, lazily-built [`ChatToolsCtx`] for the `agent_tool_*` commands: one
/// provider registry (Modrinth + CurseForge-when-keyed) and one build output dir
/// (`<data_dir>/agent/chat`), initialized once and reused across every tool call.
#[derive(Default)]
pub struct AgentToolsState(std::sync::OnceLock<ChatToolsCtx>);

impl AgentToolsState {
    fn ctx(&self) -> ChatToolsCtx {
        self.0
            .get_or_init(|| {
                let registry = Arc::new(mc_core::modplatform::provider::ProviderRegistry::with_defaults());
                ChatToolsCtx::new(registry, data_dir().join("agent").join("chat"))
            })
            .clone()
    }
}

const DEEP_DIAGNOSIS_MAX_SESSIONS: usize = 3;
const DEEP_DIAGNOSIS_MAX_TRIALS: u32 = 3;
const DEEP_DIAGNOSIS_LAUNCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticTrialOutcome {
    Stable,
    Crashed,
    LaunchError,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct DiagnosticTrialAnalysis {
    pub category: String,
    pub reason: String,
    pub matched: Option<String>,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct DiagnosticTrialResult {
    pub trial_number: u32,
    pub outcome: DiagnosticTrialOutcome,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub operations: Vec<DiagnosticTrialOperation>,
    pub log_tail: String,
    pub analysis: Option<DiagnosticTrialAnalysis>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct StartDeepDiagnosisOutput {
    pub session_id: String,
    pub baseline: DiagnosticTrialResult,
    pub max_trials: u32,
    pub sandbox_scope: String,
}
#[derive(Debug, Clone, Deserialize, specta::Type)]
pub struct RunDiagnosticTrialArgs {
    pub session_id: String,
    pub operations: Vec<DiagnosticTrialOperation>,
}
#[derive(Debug, Clone, Deserialize, specta::Type)]
pub struct FinishDeepDiagnosisArgs {
    pub session_id: String,
}
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct FinishDeepDiagnosisOutput {
    pub session_id: String,
    pub trials: Vec<DiagnosticTrialResult>,
    pub cleaned: bool,
}

struct DiagnosticSession {
    source_root: PathBuf,
    instance_id: String,
    session_root: PathBuf,
    baseline: DiagnosticSandboxSnapshot,
    next_trial: u32,
    running: bool,
    results: Vec<DiagnosticTrialResult>,
}
#[derive(Default)]
struct DiagnosticSessionBook {
    sessions: HashMap<String, DiagnosticSession>,
}

impl DiagnosticSessionBook {
    fn insert(&mut self, id: String, source_root: PathBuf, instance_id: String, session_root: PathBuf, baseline: DiagnosticSandboxSnapshot) -> CmdResult<()> {
        if self.sessions.len() >= DEEP_DIAGNOSIS_MAX_SESSIONS {
            return Err("too many active deep-diagnosis sessions; finish an existing session first".into());
        }
        self.sessions.insert(id, DiagnosticSession { source_root, instance_id, session_root, baseline, next_trial: 0, running: true, results: Vec::new() });
        Ok(())
    }
    fn reserve_trial(&mut self, session_id: &str, source_root: &Path, instance_id: &str) -> CmdResult<(u32, DiagnosticSandboxSnapshot, PathBuf)> {
        let session = self.sessions.get_mut(session_id).ok_or("unknown or expired deep-diagnosis session")?;
        if session.source_root != source_root || session.instance_id != instance_id {
            return Err("deep-diagnosis session is bound to a different instance".into());
        }
        if session.running {
            return Err("a diagnostic trial is already running for this session".into());
        }
        if session.next_trial >= DEEP_DIAGNOSIS_MAX_TRIALS {
            return Err(format!("deep-diagnosis session allows at most {DEEP_DIAGNOSIS_MAX_TRIALS} hypothesis trials"));
        }
        session.next_trial += 1;
        session.running = true;
        Ok((session.next_trial, session.baseline.clone(), session.session_root.clone()))
    }
    fn complete_trial(&mut self, id: &str, result: DiagnosticTrialResult) -> CmdResult<()> {
        let session = self.sessions.get_mut(id).ok_or("unknown or expired deep-diagnosis session")?;
        session.running = false;
        session.results.push(result);
        Ok(())
    }
    fn abort_trial(&mut self, id: &str) {
        if let Some(session) = self.sessions.get_mut(id) {
            session.running = false;
        }
    }
    fn finish(&mut self, id: &str, source_root: &Path, instance_id: &str) -> CmdResult<DiagnosticSession> {
        let session = self.sessions.get(id).ok_or("unknown or expired deep-diagnosis session")?;
        if session.source_root != source_root || session.instance_id != instance_id {
            return Err("deep-diagnosis session is bound to a different instance".into());
        }
        if session.running {
            return Err("cannot finish a deep-diagnosis session while a trial is running".into());
        }
        Ok(self.sessions.remove(id).expect("checked above"))
    }
}

#[derive(Default)]
pub struct DeepDiagnosisState(Mutex<DiagnosticSessionBook>);

fn canonical_bound_root(root: &str) -> CmdResult<(paths::GamePaths, PathBuf)> {
    let paths = root_paths(root);
    let canonical = paths.root().canonicalize().map_err(err)?;
    Ok((paths, canonical))
}
fn next_diagnostic_session_id() -> String {
    format!("diag-{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos())
}

fn diagnostic_error(trial_number: u32, operations: Vec<DiagnosticTrialOperation>, started: std::time::Instant, message: String) -> DiagnosticTrialResult {
    DiagnosticTrialResult { trial_number, outcome: DiagnosticTrialOutcome::LaunchError, exit_code: None, elapsed_ms: started.elapsed().as_millis() as u64, operations, log_tail: message, analysis: None }
}

async fn run_deep_diagnostic_launch(source_paths: &paths::GamePaths, snapshot: &DiagnosticSandboxSnapshot, trial_number: u32, operations: Vec<DiagnosticTrialOperation>) -> DiagnosticTrialResult {
    let started = std::time::Instant::now();
    let game_dir = snapshot.paths.version_dir(&snapshot.instance_id);
    let home = game_dir.join(".diagnostic-home");
    let temp = game_dir.join(".diagnostic-tmp");
    let natives = game_dir.join(".diagnostic-natives");
    let _ = std::fs::create_dir_all(&home);
    let _ = std::fs::create_dir_all(&temp);
    let dl = match make_downloader() {
        Ok(dl) => dl,
        Err(message) => return diagnostic_error(trial_number, operations, started, message),
    };
    let spec = LaunchSpec {
        instance: Instance::new(&snapshot.instance_id, source_paths.root()),
        session: auth::offline_session("kobeMC-Diagnostic"),
        java_path: None,
        launcher_name: format!("{LAUNCHER_NAME}-diagnostic"),
        launcher_version: LAUNCHER_VERSION.to_string(),
        online: false,
        runtimes_dir: None,
        global_java_path: settings_global().java_path.filter(|path| !path.is_empty()).map(PathBuf::from),
        extra_jvm_args: vec![format!("-Duser.home={}", home.to_string_lossy()), format!("-Djava.io.tmpdir={}", temp.to_string_lossy()), "-Dkobemc.diagnostic=true".into()],
        server_override: None,
        game_dir_override: Some(game_dir),
        natives_dir_override: Some(natives),
    };
    let mut child = match launch::launch(spec, &dl, None).await {
        Ok(child) => child,
        Err(error) => return diagnostic_error(trial_number, operations, started, error.to_string()),
    };
    let status = tokio::time::timeout(DEEP_DIAGNOSIS_LAUNCH_TIMEOUT, child.wait()).await;
    let (outcome, exit_code, log_tail) = match status {
        Ok(Ok(status)) => (DiagnosticTrialOutcome::Crashed, status.code(), String::new()),
        Ok(Err(error)) => (DiagnosticTrialOutcome::LaunchError, None, error.to_string()),
        Err(_) => {
            let _ = child.kill().await;
            (DiagnosticTrialOutcome::Stable, None, String::new())
        }
    };
    let analysis = if matches!(outcome, DiagnosticTrialOutcome::Crashed) {
        mc_core::diagnostics::analyze_exit(exit_code.unwrap_or(-1), &log_tail).map(|analysis| DiagnosticTrialAnalysis { category: analysis.category.slug().to_string(), reason: analysis.reason, matched: analysis.matched, suggestions: analysis.suggestions })
    } else {
        None
    };
    DiagnosticTrialResult { trial_number, outcome, exit_code, elapsed_ms: started.elapsed().as_millis() as u64, operations, log_tail, analysis }
}

/// Search Modrinth for modpacks usable as a base pack.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_search_base_modpacks(state: State<'_, AgentToolsState>, args: SearchBaseModpacksArgs) -> CmdResult<SearchBaseModpacksOutput> {
    tool_search_base_modpacks(&state.ctx(), args).await.map_err(err)
}

/// Inspect a base modpack: its bundled mods and the feature areas it covers.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_inspect_base_modpack(state: State<'_, AgentToolsState>, args: InspectBaseModpackArgs) -> CmdResult<InspectBaseModpackOutput> {
    tool_inspect_base_modpack(&state.ctx(), args).await.map_err(err)
}

/// Search all registered providers for individual mods.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_search_mods(state: State<'_, AgentToolsState>, args: SearchModsArgs) -> CmdResult<SearchModsOutput> {
    tool_search_mods(&state.ctx(), args).await.map_err(err)
}

/// Get one mod's metadata plus the versions available for a target.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_mod_get_detail(state: State<'_, AgentToolsState>, args: ModGetDetailArgs) -> CmdResult<ModGetDetailOutput> {
    tool_mod_get_detail(&state.ctx(), args).await.map_err(err)
}

/// Resolve project ids into concrete, download-ready files (walks dependencies).
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_resolve_mods(state: State<'_, AgentToolsState>, args: ResolveModsArgs) -> CmdResult<ResolveModsOutput> {
    tool_resolve_mods(&state.ctx(), args).await.map_err(err)
}

/// Validate the exact selected versions and dependency/conflict graph without writing.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_validate_modpack_plan(state: State<'_, AgentToolsState>, args: ValidateModpackPlanArgs) -> CmdResult<ValidateModpackPlanOutput> {
    tool_validate_modpack_plan(&state.ctx(), args).await.map_err(err)
}

/// Deterministically build + verify a `.mrpack` from a base pack (or scratch) plus
/// extra mods. Writes to disk; the TS loop must gate this behind user confirmation.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_build_modpack(state: State<'_, AgentToolsState>, args: BuildModpackArgs) -> CmdResult<BuildModpackOutput> {
    tool_build_modpack(&state.ctx(), args).await.map_err(err)
}

/// Install an agent-built `.mrpack` (from the chat sandbox dir) into `root` as a
/// playable instance. Path sandboxing lives in the mc-core tool; the engine here
/// is the same import engine `import_modpack` uses.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_install_modpack(state: State<'_, AgentToolsState>, root: String, args: InstallModpackArgs) -> CmdResult<InstallModpackOutput> {
    use mc_core::modpack::import::ImportEngine;
    let dl = make_downloader()?;
    let engine = ImportEngine::with_defaults(dl, make_registry());
    let paths = root_paths(&root);
    let out = tool_install_modpack(&state.ctx(), &engine, paths.root(), args).await.map_err(err)?;
    best_effort_refresh_wiki_cache(&paths, &out.instance_id).await;
    Ok(out)
}

/// Read-only lean instance list for the agent (id / name / mc_version / loader).
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_list_instances(root: String) -> CmdResult<ListInstancesOutput> {
    tool_list_instances(&root_paths(&root)).map_err(err)
}

/// Diagnose one host-bound installed instance. The model never supplies root/id.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_diagnose_instance(root: String, id: String, args: DiagnoseInstanceArgs) -> CmdResult<DiagnoseInstanceOutput> {
    tool_diagnose_instance(&root_paths(&root), &id, args).await.map_err(err)
}

#[tauri::command]
#[specta::specta]
pub async fn agent_tool_start_deep_diagnosis(state: State<'_, DeepDiagnosisState>, running: State<'_, RunningGames>, root: String, id: String) -> CmdResult<StartDeepDiagnosisOutput> {
    if running.is_running(&id) {
        return Err("stop the installed instance before starting deep diagnosis".into());
    }
    let (source_paths, canonical_root) = canonical_bound_root(&root)?;
    let session_id = next_diagnostic_session_id();
    let session_root = data_dir().join("agent").join("diagnostics").join(&session_id);
    let baseline = create_diagnostic_snapshot(&source_paths, &id, &session_root.join("baseline")).map_err(err)?;
    let baseline_trial = clone_diagnostic_snapshot(&baseline, &session_root.join("trials").join("trial-0")).map_err(err)?;
    state.0.lock().map_err(err)?.insert(session_id.clone(), canonical_root, id, session_root, baseline)?;
    let result = run_deep_diagnostic_launch(&source_paths, &baseline_trial, 0, Vec::new()).await;
    state.0.lock().map_err(err)?.complete_trial(&session_id, result.clone())?;
    Ok(StartDeepDiagnosisOutput { session_id, baseline: result, max_trials: DEEP_DIAGNOSIS_MAX_TRIALS, sandbox_scope: "temporary instance filesystem; not OS or network isolation".into() })
}

#[tauri::command]
#[specta::specta]
pub async fn agent_tool_run_diagnostic_trial(state: State<'_, DeepDiagnosisState>, running: State<'_, RunningGames>, root: String, id: String, args: RunDiagnosticTrialArgs) -> CmdResult<DiagnosticTrialResult> {
    if args.operations.is_empty() {
        return Err("diagnostic hypothesis trial requires at least one operation".into());
    }
    if running.is_running(&id) {
        return Err("stop the installed instance before running a diagnostic trial".into());
    }
    let (source_paths, canonical_root) = canonical_bound_root(&root)?;
    let (trial_number, baseline, session_root) = state.0.lock().map_err(err)?.reserve_trial(&args.session_id, &canonical_root, &id)?;
    let trial = match (|| {
        let trial = clone_diagnostic_snapshot(&baseline, &session_root.join("trials").join(format!("trial-{trial_number}")))?;
        apply_diagnostic_operations(&trial.paths, &trial.instance_id, &args.operations)?;
        Ok::<_, mc_core::agent::tools::ChatToolError>(trial)
    })() {
        Ok(trial) => trial,
        Err(error) => {
            state.0.lock().map_err(err)?.abort_trial(&args.session_id);
            return Err(error.to_string());
        }
    };
    let result = run_deep_diagnostic_launch(&source_paths, &trial, trial_number, args.operations).await;
    state.0.lock().map_err(err)?.complete_trial(&args.session_id, result.clone())?;
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub fn agent_tool_finish_deep_diagnosis(state: State<'_, DeepDiagnosisState>, root: String, id: String, args: FinishDeepDiagnosisArgs) -> CmdResult<FinishDeepDiagnosisOutput> {
    let (_, canonical_root) = canonical_bound_root(&root)?;
    let session = state.0.lock().map_err(err)?.finish(&args.session_id, &canonical_root, &id)?;
    cleanup_diagnostic_session(&session.session_root).map_err(err)?;
    Ok(FinishDeepDiagnosisOutput { session_id: args.session_id, trials: session.results, cleaned: true })
}

/// Agent wiki tool: full-text search over the instance's local wiki corpus.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_wiki_search(root: String, args: WikiSearchArgs) -> CmdResult<WikiSearchOutput> {
    validate_agent_wiki_source_paths(&root, &args.source_paths)?;
    tool_wiki_search(args).await.map_err(err)
}

/// Open one wiki chunk returned by `agent_tool_wiki_search`.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_wiki_open(root: String, args: WikiOpenArgs) -> CmdResult<WikiOpenOutput> {
    validate_agent_wiki_source_paths(&root, &args.source_paths)?;
    tool_wiki_open(args).await.map_err(err)
}

/// Rebuild the local wiki corpus cache for an installed instance on demand.
#[tauri::command]
#[specta::specta]
pub async fn rebuild_instance_wiki_index(root: String, id: String) -> CmdResult<()> {
    let paths = root_paths(&root);
    refresh_wiki_cache_for_instance(&paths, &id).await
}

/// The local OpenRouter config (key / model / base_url) resolved from env + the
/// repo-root `.env` via [`AgentLlmConfig::from_local`].
///
/// NOTE: this hands the user's own API key to the webview so a TS agent loop can
/// call OpenRouter directly. Acceptable for a local desktop app using the user's
/// key; it never leaves this machine except to OpenRouter.
#[derive(Serialize, specta::Type)]
pub struct AgentLlmConfigDto {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

#[tauri::command]
#[specta::specta]
pub fn agent_llm_config() -> CmdResult<AgentLlmConfigDto> {
    let cfg = mc_core::agent::AgentLlmConfig::from_local(&data_dir()).map_err(err)?;
    Ok(AgentLlmConfigDto { api_key: cfg.api_key, model: cfg.model, base_url: cfg.base_url })
}

// --- local agent runtime (claude-code engine in a Node host) ------------------
//
// The webview brain can't spawn processes, so these THIN commands manage one
// `node harness-host.mjs` child: spawn it, forward stdin lines, and emit its
// stdout lines back as `agent-host://event`. The protocol peers are the webview
// (localRuntimeAdapter) and the Node host; Rust is a dumb pipe — no launcher
// logic, no message inspection.

/// One line from the Node host's stdout (a JSON protocol message), or the
/// synthetic `{"type":"host_exit"}` emitted when the child dies, so the webview
/// can fail an in-flight turn instead of hanging.
#[derive(Serialize, Clone, specta::Type)]
pub struct AgentHostEvent {
    pub line: String,
}

#[derive(Default)]
pub struct AgentHostState(Mutex<Option<std::process::Child>>);

/// Locate `packages/agent-core/bin/harness-host.mjs`: `MC_AGENT_HOST_SCRIPT`
/// env override first, then walk up from the executable (dev: target/debug/…
/// sits inside the repo). Packaged-app resource bundling is a later concern.
fn agent_host_script() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("MC_AGENT_HOST_SCRIPT") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let mut dir = std::env::current_exe().ok()?;
    while dir.pop() {
        let candidate = dir.join("packages/agent-core/bin/harness-host.mjs");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Start (or reuse) the Node agent host. Idempotent: a live child is kept.
#[tauri::command]
#[specta::specta]
pub fn agent_host_start(app: AppHandle, state: State<'_, AgentHostState>) -> CmdResult<()> {
    let mut guard = state.0.lock().map_err(err)?;
    if let Some(child) = guard.as_mut() {
        if child.try_wait().map_err(err)?.is_none() {
            return Ok(()); // already running
        }
        *guard = None;
    }
    let script = agent_host_script().ok_or_else(|| "harness-host.mjs not found (set MC_AGENT_HOST_SCRIPT)".to_string())?;
    let node = mc_core::agent::runtime::detect_local_runtime().node.ok_or_else(|| "node runtime not found on this machine".to_string())?;
    tracing::info!(target: "daemon", script = %script.display(), node = %node.path, "starting agent host");
    let mut child = std::process::Command::new(&node.path).arg(&script).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(err)?;

    // stdout lines → webview events; on EOF (child died) send a synthetic
    // host_exit so the adapter can fail fast instead of hanging a turn.
    let stdout = child.stdout.take().ok_or("agent host stdout unavailable")?;
    let app_out = app.clone();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stdout).lines().map_while(|l| l.ok()) {
            let _ = app_out.emit("agent-host://event", AgentHostEvent { line });
        }
        let _ = app_out.emit("agent-host://event", AgentHostEvent { line: "{\"type\":\"host_exit\"}".to_string() });
    });
    // stderr → daemon log (host diagnostics; harness bridge noise).
    let stderr = child.stderr.take().ok_or("agent host stderr unavailable")?;
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stderr).lines().map_while(|l| l.ok()) {
            tracing::info!(target: "daemon", "agent-host: {line}");
        }
    });

    *guard = Some(child);
    Ok(())
}

/// Forward one protocol line to the Node host's stdin.
#[tauri::command]
#[specta::specta]
pub fn agent_host_send(state: State<'_, AgentHostState>, line: String) -> CmdResult<()> {
    use std::io::Write;
    let mut guard = state.0.lock().map_err(err)?;
    let child = guard.as_mut().ok_or("agent host not running")?;
    let stdin = child.stdin.as_mut().ok_or("agent host stdin unavailable")?;
    writeln!(stdin, "{line}").map_err(err)?;
    stdin.flush().map_err(err)
}

/// Stop the Node host: ask it to dispose (kills its runtime session), close
/// stdin, then reap — force-kill only if it lingers.
#[tauri::command]
#[specta::specta]
pub fn agent_host_stop(state: State<'_, AgentHostState>) -> CmdResult<()> {
    let Some(mut child) = state.0.lock().map_err(err)?.take() else {
        return Ok(());
    };
    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        let _ = writeln!(stdin, "{{\"type\":\"dispose\"}}");
        let _ = stdin.flush();
    }
    drop(child.stdin.take()); // EOF → host's own cleanup path
    std::thread::spawn(move || {
        for _ in 0..100 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = child.kill();
    });
    Ok(())
}

/// What the local Claude Code agent path needs, per binary (None = not found).
#[derive(Serialize, specta::Type)]
pub struct LocalRuntimeStatusDto {
    pub claude_code: Option<String>,
    pub node: Option<String>,
    pub pnpm: Option<String>,
}

/// Detect the locally-installed Claude Code runtime prerequisites (settings UI).
#[tauri::command]
#[specta::specta]
pub async fn agent_runtime_detect() -> CmdResult<LocalRuntimeStatusDto> {
    // --version spawns are slow-ish; keep them off the main thread.
    let status = tokio::task::spawn_blocking(mc_core::agent::detect_local_runtime).await.map_err(err)?;
    Ok(LocalRuntimeStatusDto { claude_code: status.claude_code.map(|b| b.version), node: status.node.map(|b| b.version), pnpm: status.pnpm.map(|b| b.version) })
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
