use super::*;

/// A published agent chat transcript: its short id + the public fetch URL.
#[derive(serde::Serialize, specta::Type)]
pub struct SharedConversation {
    pub id: String,
    pub url: String,
}

/// Publish the current agent chat transcript to the deployed mc-server for public
/// sharing (always cloud — no local fallback). Requires a signed-in kobeMC
/// account: uses the shared managed client so the better-auth session cookie is
/// sent. `payload_json` is the JSON.stringify'd transcript (String, to avoid
/// exporting a recursive `serde_json::Value` through specta).
#[tauri::command]
#[specta::specta]
pub async fn agent_share_conversation(
    client: tauri::State<'_, mc_core::server::ServerClient>,
    payload_json: String,
) -> CmdResult<SharedConversation> {
    let payload: serde_json::Value = serde_json::from_str(&payload_json).map_err(err)?;
    let (id, url) = client.share_conversation(&payload).await.map_err(err)?;
    Ok(SharedConversation { id, url })
}

// --- agent conversation history (cloud sync; authed) -------------------------
// Thin wrappers over `ServerClient::agent_history_*`. Records travel as JSON
// strings (same reason as `agent_share_conversation`: no recursive Value in specta).

/// List the signed-in user's archived conversations (heads only, newest first).
#[tauri::command]
#[specta::specta]
pub async fn agent_history_list(
    client: tauri::State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Vec<mc_core::server::AgentConversationHead>> {
    client.agent_history_list().await.map_err(err)
}

/// Fetch one archived conversation's full record, as a JSON string.
#[tauri::command]
#[specta::specta]
pub async fn agent_history_get(
    client: tauri::State<'_, mc_core::server::ServerClient>,
    id: String,
) -> CmdResult<String> {
    let record = client.agent_history_get(&id).await.map_err(err)?;
    serde_json::to_string(&record).map_err(err)
}

/// Upsert one conversation record (JSON string) into the user's cloud history.
#[tauri::command]
#[specta::specta]
pub async fn agent_history_put(
    client: tauri::State<'_, mc_core::server::ServerClient>,
    id: String,
    record_json: String,
) -> CmdResult<()> {
    let record: serde_json::Value = serde_json::from_str(&record_json).map_err(err)?;
    client.agent_history_put(&id, &record).await.map_err(err)
}

/// Delete one archived conversation from the user's cloud history.
#[tauri::command]
#[specta::specta]
pub async fn agent_history_delete(
    client: tauri::State<'_, mc_core::server::ServerClient>,
    id: String,
) -> CmdResult<()> {
    client.agent_history_delete(&id).await.map_err(err)
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
                let registry =
                    Arc::new(mc_core::modplatform::provider::ProviderRegistry::with_defaults());
                ChatToolsCtx::new(registry, data_dir().join("agent").join("chat"))
            })
            .clone()
    }
}

/// Search Modrinth for modpacks usable as a base pack.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_search_base_modpacks(
    state: State<'_, AgentToolsState>,
    args: SearchBaseModpacksArgs,
) -> CmdResult<SearchBaseModpacksOutput> {
    tool_search_base_modpacks(&state.ctx(), args).await.map_err(err)
}

/// Inspect a base modpack: its bundled mods and the feature areas it covers.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_inspect_base_modpack(
    state: State<'_, AgentToolsState>,
    args: InspectBaseModpackArgs,
) -> CmdResult<InspectBaseModpackOutput> {
    tool_inspect_base_modpack(&state.ctx(), args).await.map_err(err)
}

/// Search all registered providers for individual mods.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_search_mods(
    state: State<'_, AgentToolsState>,
    args: SearchModsArgs,
) -> CmdResult<SearchModsOutput> {
    tool_search_mods(&state.ctx(), args).await.map_err(err)
}

/// Get one mod's metadata plus the versions available for a target.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_mod_get_detail(
    state: State<'_, AgentToolsState>,
    args: ModGetDetailArgs,
) -> CmdResult<ModGetDetailOutput> {
    tool_mod_get_detail(&state.ctx(), args).await.map_err(err)
}

/// Resolve project ids into concrete, download-ready files (walks dependencies).
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_resolve_mods(
    state: State<'_, AgentToolsState>,
    args: ResolveModsArgs,
) -> CmdResult<ResolveModsOutput> {
    tool_resolve_mods(&state.ctx(), args).await.map_err(err)
}

/// Deterministically build + verify a `.mrpack` from a base pack (or scratch) plus
/// extra mods. Writes to disk; the TS loop must gate this behind user confirmation.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_build_modpack(
    state: State<'_, AgentToolsState>,
    args: BuildModpackArgs,
) -> CmdResult<BuildModpackOutput> {
    tool_build_modpack(&state.ctx(), args).await.map_err(err)
}

/// Install an agent-built `.mrpack` (from the chat sandbox dir) into `root` as a
/// playable instance. Path sandboxing lives in the mc-core tool; the engine here
/// is the same import engine `import_modpack` uses.
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_install_modpack(
    state: State<'_, AgentToolsState>,
    root: String,
    args: InstallModpackArgs,
) -> CmdResult<InstallModpackOutput> {
    use mc_core::modpack::import::ImportEngine;
    let dl = make_downloader()?;
    let engine = ImportEngine::with_defaults(dl, make_registry());
    let paths = root_paths(&root);
    tool_install_modpack(&state.ctx(), &engine, paths.root(), args)
        .await
        .map_err(err)
}

/// Read-only lean instance list for the agent (id / name / mc_version / loader).
#[tauri::command]
#[specta::specta]
pub async fn agent_tool_list_instances(root: String) -> CmdResult<ListInstancesOutput> {
    tool_list_instances(&root_paths(&root)).map_err(err)
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
    Ok(AgentLlmConfigDto {
        api_key: cfg.api_key,
        model: cfg.model,
        base_url: cfg.base_url,
    })
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
    let script = agent_host_script()
        .ok_or_else(|| "harness-host.mjs not found (set MC_AGENT_HOST_SCRIPT)".to_string())?;
    let node = mc_core::agent::runtime::detect_local_runtime()
        .node
        .ok_or_else(|| "node runtime not found on this machine".to_string())?;
    tracing::info!(target: "daemon", script = %script.display(), node = %node.path, "starting agent host");
    let mut child = std::process::Command::new(&node.path)
        .arg(&script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(err)?;

    // stdout lines → webview events; on EOF (child died) send a synthetic
    // host_exit so the adapter can fail fast instead of hanging a turn.
    let stdout = child.stdout.take().ok_or("agent host stdout unavailable")?;
    let app_out = app.clone();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stdout).lines().map_while(|l| l.ok()) {
            let _ = app_out.emit("agent-host://event", AgentHostEvent { line });
        }
        let _ = app_out.emit(
            "agent-host://event",
            AgentHostEvent { line: "{\"type\":\"host_exit\"}".to_string() },
        );
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
    let status = tokio::task::spawn_blocking(mc_core::agent::detect_local_runtime)
        .await
        .map_err(err)?;
    Ok(LocalRuntimeStatusDto {
        claude_code: status.claude_code.map(|b| b.version),
        node: status.node.map(|b| b.version),
        pnpm: status.pnpm.map(|b| b.version),
    })
}
