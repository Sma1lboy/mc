use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};

use mc_core::agent::{
    AgentEntry, AgentInputKind, AgentIntentKind, AgentInterruptKind, AgentMessageKind, AgentPhase,
    AgentRunSnapshot, AgentSessionStore, AgentSessionSummary, AgentStatus, ApprovalKind,
    ApprovalRequest, BuildRestrictions, EXPORT_MRPACK_ARTIFACT_TOOL, UserDecisionKind,
};

use crate::data_dir;

mod render;

#[cfg(test)]
mod tests;

#[cfg(test)]
use render::{
    customization_unresolved_request_lines, execution_next_step_command,
    latest_approval_clarification_message, pending_approval_next_step_lines,
};
use render::{print_agent_session_list, print_agent_snapshot};

#[derive(Subcommand)]
pub(crate) enum AgentAction {
    /// Start the main agent; supported build-modpack requests stop at an approval gate.
    #[command(alias = "plan")]
    Start {
        /// Natural-language user request.
        prompt: String,
        /// Optional session id. Defaults to a generated agent-run-* id.
        #[arg(long)]
        session_id: Option<String>,
        /// Override OpenRouter model for this run. Defaults to
        /// MC_AGENT_OPENROUTER_MODEL / OPENROUTER_MODEL / the core default.
        #[arg(long)]
        model: Option<String>,
        /// Print where the local API key should be placed and exit.
        #[arg(long)]
        show_key_path: bool,
        /// Print the full raw AgentRunSnapshot JSON.
        #[arg(long)]
        json: bool,
        /// UI surface that launched this agent run.
        #[arg(long, value_enum, default_value_t = AgentStartSurface::Home)]
        surface: AgentStartSurface,
    },
    /// Show a saved local agent session snapshot.
    Show {
        #[arg(long)]
        session_id: String,
        /// Print the full raw AgentRunSnapshot JSON.
        #[arg(long)]
        json: bool,
    },
    /// Continue a saved agent session with a natural-language user response.
    Continue {
        #[arg(long)]
        session_id: String,
        /// Natural-language response; the agent routes it to approve/revise/cancel.
        #[arg(long)]
        message: String,
        /// Print the full raw AgentRunSnapshot JSON.
        #[arg(long)]
        json: bool,
    },
    /// Export an approved modpack build as a real .mrpack file.
    #[command(name = "export", alias = "execute")]
    Execute {
        #[arg(long)]
        session_id: String,
        /// Destination .mrpack path.
        #[arg(long)]
        output: PathBuf,
        /// Print the full raw AgentRunSnapshot JSON.
        #[arg(long)]
        json: bool,
    },
    /// Apply a deterministic execution outcome to a session without calling the model.
    ExecSmoke {
        #[arg(long)]
        session_id: String,
        #[arg(long, value_enum, default_value_t = AgentExecSmokeOutcome::Ready)]
        outcome: AgentExecSmokeOutcome,
        /// Optional reason used by retry/failed/blocked smoke outcomes.
        #[arg(long)]
        reason: Option<String>,
        /// Print the full raw AgentRunSnapshot JSON.
        #[arg(long)]
        json: bool,
    },
    /// List saved local agent sessions.
    List {
        /// Print raw session summary JSON.
        #[arg(long)]
        json: bool,
        /// Show all sessions in human-readable mode.
        #[arg(long)]
        all: bool,
        /// Maximum sessions to show in human-readable mode.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Delete a saved local agent session.
    Delete {
        #[arg(long)]
        session_id: String,
        /// Print raw deletion JSON.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) async fn cmd_agent(action: &AgentAction) -> Result<()> {
    match action {
        AgentAction::Start {
            prompt,
            session_id,
            model,
            show_key_path,
            json,
            surface,
        } => {
            cmd_agent_start(
                prompt,
                session_id.clone(),
                model.clone(),
                *show_key_path,
                *json,
                *surface,
            )
            .await
        }
        AgentAction::Show { session_id, json } => cmd_agent_show(session_id, *json),
        AgentAction::Continue {
            session_id,
            message,
            json,
        } => cmd_agent_continue(session_id, message, *json).await,
        AgentAction::Execute {
            session_id,
            output,
            json,
        } => cmd_agent_execute(session_id, output, *json).await,
        AgentAction::ExecSmoke {
            session_id,
            outcome,
            reason,
            json,
        } => cmd_agent_exec_smoke(session_id, *outcome, reason.as_deref(), *json),
        AgentAction::List { json, all, limit } => cmd_agent_list(*json, *all, *limit),
        AgentAction::Delete { session_id, json } => cmd_agent_delete(session_id, *json),
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum AgentExecSmokeOutcome {
    Ready,
    Completed,
    Retry,
    Failed,
    BlockedCustomization,
    BlockedBasePack,
    BlockedRequirements,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum AgentStartSurface {
    Home,
}

async fn cmd_agent_start(
    prompt: &str,
    session_id: Option<String>,
    model: Option<String>,
    show_key_path: bool,
    json: bool,
    surface: AgentStartSurface,
) -> Result<()> {
    let dir = data_dir();
    if show_key_path {
        println!("OpenRouter key lookup order:");
        println!("  1. OPENROUTER_API_KEY");
        println!("  2. repository root .env:");
        for path in mc_core::agent::AgentLlmConfig::local_env_paths(&dir) {
            println!("     - {}", path.display());
        }
        return Ok(());
    }

    let agent = local_agent_runtime(&dir, model)?;
    let entry = agent_entry_from_start_flags(surface)?;
    let mut snapshot = agent.start_new_run_with_entry(prompt, entry).await?;
    if let Some(session_id) = session_id.filter(|s| !s.trim().is_empty()) {
        snapshot.id = session_id;
    }
    let store = AgentSessionStore::new(&dir);
    persist_and_print_agent_snapshot(&store, snapshot, json).map(|_| ())
}

fn agent_entry_from_start_flags(surface: AgentStartSurface) -> Result<AgentEntry> {
    match surface {
        AgentStartSurface::Home => Ok(AgentEntry::Home),
    }
}

fn local_agent_runtime(
    dir: &Path,
    model: Option<String>,
) -> Result<mc_core::agent::MainAgentRuntime> {
    let mut cfg = mc_core::agent::AgentLlmConfig::from_local(dir).with_context(|| {
        "Failed to read the OpenRouter API key; set OPENROUTER_API_KEY or write it to the repository root .env"
    })?;
    if let Some(model) = model.filter(|s| !s.trim().is_empty()) {
        cfg.model = model;
    }

    let llm = mc_core::agent::AgentLlmClient::new(cfg)?;
    Ok(mc_core::agent::MainAgentRuntime::new(llm))
}

fn cmd_agent_show(session_id: &str, json: bool) -> Result<()> {
    let dir = data_dir();
    cmd_agent_show_with_dir(&dir, session_id, json).map(|_| ())
}

fn cmd_agent_show_with_dir(dir: &Path, session_id: &str, json: bool) -> Result<AgentRunSnapshot> {
    let store = AgentSessionStore::new(dir);
    let snapshot = load_agent_snapshot(&store, session_id)?;
    print_agent_snapshot(&snapshot, json)?;
    Ok(snapshot)
}

async fn cmd_agent_continue(session_id: &str, message: &str, json: bool) -> Result<()> {
    let user_message = message.trim();
    if user_message.is_empty() {
        anyhow::bail!("--message must not be empty");
    }

    let dir = data_dir();
    let store = AgentSessionStore::new(&dir);
    let snapshot = load_agent_snapshot(&store, session_id)?;
    ensure_session_can_continue(&snapshot)?;
    let agent = local_agent_runtime(&dir, None)?;
    cmd_agent_continue_snapshot_with_runtime(&dir, &agent, snapshot, user_message, json)
        .await
        .map(|_| ())
}

#[cfg(test)]
async fn cmd_agent_continue_with_runtime(
    dir: &Path,
    agent: &mc_core::agent::MainAgentRuntime,
    session_id: &str,
    user_message: &str,
    json: bool,
) -> Result<AgentRunSnapshot> {
    let store = AgentSessionStore::new(dir);
    let snapshot = load_agent_snapshot(&store, session_id)?;
    cmd_agent_continue_snapshot_with_runtime(dir, agent, snapshot, user_message, json).await
}

async fn cmd_agent_continue_snapshot_with_runtime(
    dir: &Path,
    agent: &mc_core::agent::MainAgentRuntime,
    snapshot: AgentRunSnapshot,
    user_message: &str,
    json: bool,
) -> Result<AgentRunSnapshot> {
    ensure_session_can_continue(&snapshot)?;
    let store = AgentSessionStore::new(dir);
    let next = agent
        .continue_from_user_message(snapshot, user_message)
        .await?;
    persist_and_print_agent_snapshot(&store, next, json)
}

async fn cmd_agent_execute(session_id: &str, output: &Path, json: bool) -> Result<()> {
    let dir = data_dir();
    cmd_agent_execute_with_dir(&dir, session_id, output, json)
        .await
        .map(|_| ())
}

async fn cmd_agent_execute_with_dir(
    dir: &Path,
    session_id: &str,
    output: &Path,
    json: bool,
) -> Result<AgentRunSnapshot> {
    let store = AgentSessionStore::new(dir);
    let snapshot = load_agent_snapshot(&store, session_id)?;
    if execution_completed(&snapshot) {
        let next = copy_completed_artifact_to_output(snapshot, output)?;
        return persist_and_print_agent_snapshot(&store, next, json);
    }
    ensure_session_is_executable(&snapshot)?;
    let agent = deterministic_agent_runtime()?;
    let next = agent
        .execute_tool(
            snapshot,
            EXPORT_MRPACK_ARTIFACT_TOOL,
            serde_json::json!({
                "output_path": output.to_string_lossy().to_string(),
            }),
        )
        .await?;
    persist_and_print_agent_snapshot(&store, next, json)
}

fn persist_and_print_agent_snapshot(
    store: &AgentSessionStore,
    mut snapshot: AgentRunSnapshot,
    json: bool,
) -> Result<AgentRunSnapshot> {
    snapshot.push_trace("saved local agent session");
    store.save_snapshot(&snapshot)?;
    print_agent_snapshot(&snapshot, json)?;
    Ok(snapshot)
}

fn load_agent_snapshot(store: &AgentSessionStore, session_id: &str) -> Result<AgentRunSnapshot> {
    match store.load_snapshot(session_id) {
        Ok(snapshot) => Ok(snapshot),
        Err(mc_core::CoreError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            anyhow::bail!(
                "Session '{session_id}' was not found. Run `mc agent list` to see existing sessions."
            )
        }
        Err(err) => Err(err.into()),
    }
}

fn ensure_session_can_continue(snapshot: &AgentRunSnapshot) -> Result<()> {
    if snapshot.status == AgentStatus::Completed || snapshot.phase == AgentPhase::Completed {
        anyhow::bail!("This session is completed and cannot be continued.");
    }
    Ok(())
}

#[cfg(test)]
fn default_agent_output_path(data_dir: &Path, session_id: &str) -> PathBuf {
    data_dir
        .join("agent")
        .join("artifacts")
        .join(format!("{session_id}.mrpack"))
}

fn ensure_session_is_executable(snapshot: &AgentRunSnapshot) -> Result<()> {
    if snapshot.status == AgentStatus::Running
        && matches!(
            snapshot.phase,
            AgentPhase::ExecutionReady | AgentPhase::Executing
        )
        && snapshot.approved_build.is_some()
    {
        return Ok(());
    }
    anyhow::bail!(
        "This session does not have an approved executable plan yet. Complete the approval gates first."
    )
}

fn execution_completed(snapshot: &AgentRunSnapshot) -> bool {
    snapshot.status == AgentStatus::Completed
        || snapshot.phase == AgentPhase::Completed
        || snapshot.execution.as_ref().is_some_and(|execution| {
            execution.status == mc_core::agent::AgentExecutionStatus::Completed
        })
}

fn copy_completed_artifact_to_output(
    mut snapshot: AgentRunSnapshot,
    output: &Path,
) -> Result<AgentRunSnapshot> {
    let source = completed_artifact_path(&snapshot).ok_or_else(|| {
        anyhow::anyhow!("This session is completed, but no copyable artifact path was recorded.")
    })?;
    if !source.exists() {
        anyhow::bail!(
            "This session is completed, but the recorded artifact path does not exist: {}",
            source.display()
        );
    }
    if source != output {
        if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create output directory: {}", parent.display())
            })?;
        }
        std::fs::copy(&source, output).with_context(|| {
            format!(
                "Failed to copy completed artifact: {} -> {}",
                source.display(),
                output.display()
            )
        })?;
    }
    update_completed_manifest_output(&mut snapshot, output)?;
    snapshot.push_trace("copied completed agent artifact to requested output");
    Ok(snapshot)
}

fn completed_artifact_path(snapshot: &AgentRunSnapshot) -> Option<PathBuf> {
    snapshot
        .execution
        .as_ref()
        .and_then(|execution| execution.manifest.as_ref())
        .and_then(|manifest| manifest.get("output_path"))
        .and_then(|path| path.as_str())
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

fn update_completed_manifest_output(snapshot: &mut AgentRunSnapshot, output: &Path) -> Result<()> {
    let Some(manifest) = snapshot
        .execution
        .as_mut()
        .and_then(|execution| execution.manifest.as_mut())
    else {
        return Ok(());
    };
    let Some(obj) = manifest.as_object_mut() else {
        return Ok(());
    };
    obj.insert(
        "output_path".to_string(),
        serde_json::json!(output.to_string_lossy().to_string()),
    );
    let size = std::fs::metadata(output)
        .with_context(|| format!("Failed to read output file metadata: {}", output.display()))?
        .len();
    obj.insert("output_size".to_string(), serde_json::json!(size));
    Ok(())
}

fn deterministic_agent_runtime() -> Result<mc_core::agent::MainAgentRuntime> {
    let cfg = mc_core::agent::AgentLlmConfig::new("deterministic-execution");
    let llm = mc_core::agent::AgentLlmClient::new(cfg)?;
    Ok(mc_core::agent::MainAgentRuntime::new(llm))
}

fn cmd_agent_exec_smoke(
    session_id: &str,
    outcome: AgentExecSmokeOutcome,
    reason: Option<&str>,
    json: bool,
) -> Result<()> {
    let dir = data_dir();
    let store = AgentSessionStore::new(&dir);
    let snapshot = load_agent_snapshot(&store, session_id)?;
    let manifest = exec_smoke_manifest(outcome, reason);
    let next = mc_core::agent::continue_after_execution_manifest_result(snapshot, manifest)?;
    persist_and_print_agent_snapshot(&store, next, json).map(|_| ())
}

fn cmd_agent_list(json: bool, all: bool, limit: usize) -> Result<()> {
    let sessions = AgentSessionStore::new(data_dir()).list_sessions()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
    } else {
        print_agent_session_list(&sessions, all, limit);
    }
    Ok(())
}

fn cmd_agent_delete(session_id: &str, json: bool) -> Result<()> {
    let deleted = AgentSessionStore::new(data_dir()).delete_session(session_id)?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "session_id": session_id, "deleted": deleted })
        );
    } else {
        println!("session: {session_id}");
        println!("deleted: {deleted}");
    }
    Ok(())
}

fn exec_smoke_manifest(outcome: AgentExecSmokeOutcome, reason: Option<&str>) -> serde_json::Value {
    let reason = reason
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(match outcome {
            AgentExecSmokeOutcome::Ready => "execution manifest compiled",
            AgentExecSmokeOutcome::Completed => "mrpack smoke execution completed",
            AgentExecSmokeOutcome::Retry => "source timed out",
            AgentExecSmokeOutcome::Failed => "unrecoverable execution error",
            AgentExecSmokeOutcome::BlockedCustomization => {
                "extra mod source metadata is incomplete"
            }
            AgentExecSmokeOutcome::BlockedBasePack => "base archive is missing modrinth.index.json",
            AgentExecSmokeOutcome::BlockedRequirements => "target compatibility mismatch",
        });
    match outcome {
        AgentExecSmokeOutcome::Ready => serde_json::json!({
            "status": "ready",
            "format": "mrpack",
            "output_index": { "files": [] },
            "reason": reason,
        }),
        AgentExecSmokeOutcome::Completed => serde_json::json!({
            "status": "completed",
            "format": "mrpack",
            "output_path": "smoke://agent-output.mrpack",
            "reason": reason,
        }),
        AgentExecSmokeOutcome::Retry => serde_json::json!({
            "status": "retry",
            "retryable": true,
            "error_kind": "network_timeout",
            "reason": reason,
        }),
        AgentExecSmokeOutcome::Failed => serde_json::json!({
            "status": "failed",
            "retryable": false,
            "reason": reason,
        }),
        AgentExecSmokeOutcome::BlockedCustomization => serde_json::json!({
            "status": "blocked",
            "replan_phase": "confirm_customization_approval",
            "blocked": [{
                "title": "extra mods",
                "reason": reason,
            }],
        }),
        AgentExecSmokeOutcome::BlockedBasePack => serde_json::json!({
            "status": "blocked",
            "replan_phase": "base_pack",
            "blocked": [{
                "title": "base pack",
                "reason": reason,
            }],
        }),
        AgentExecSmokeOutcome::BlockedRequirements => serde_json::json!({
            "status": "blocked",
            "replan_phase": "requirements",
            "blocked": [{
                "title": "target",
                "reason": reason,
            }],
        }),
    }
}
