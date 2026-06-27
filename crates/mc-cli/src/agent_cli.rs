use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};

use mc_core::agent::{
    AgentIntentKind, AgentMessageKind, AgentPhase, AgentRunSnapshot, AgentSessionSummary,
    AgentStatus, ApprovalKind, ApprovalRequest, BuildRestrictions,
};

use crate::data_dir;

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
        /// Override model for this run. Defaults to MC_AGENT_OPENAI_MODEL /
        /// OPENAI_MODEL / the core default.
        #[arg(long)]
        model: Option<String>,
        /// Print where the local API key should be placed and exit.
        #[arg(long)]
        show_key_path: bool,
        /// Print the full raw AgentRunSnapshot JSON.
        #[arg(long)]
        json: bool,
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
    /// Execute an approved modpack build and write a real .mrpack output.
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
        } => {
            cmd_agent_start(
                prompt,
                session_id.clone(),
                model.clone(),
                *show_key_path,
                *json,
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

async fn cmd_agent_start(
    prompt: &str,
    session_id: Option<String>,
    model: Option<String>,
    show_key_path: bool,
    json: bool,
) -> Result<()> {
    let dir = data_dir();
    if show_key_path {
        println!("OpenAI key lookup order:");
        println!("  1. OPENAI_API_KEY");
        println!("  2. nearest .env files:");
        for path in mc_core::agent::AgentLlmConfig::local_env_paths(&dir) {
            if path.exists() {
                println!("     - {}", path.display());
            }
        }
        return Ok(());
    }

    let agent = local_agent_runtime(&dir, model)?;
    let mut snapshot = agent.start_new_run(prompt).await?;
    if let Some(session_id) = session_id.filter(|s| !s.trim().is_empty()) {
        snapshot.id = session_id;
    }
    snapshot.push_trace("saved local agent session");
    mc_core::agent::AgentSessionStore::new(&dir).save_snapshot(&snapshot)?;
    print_agent_snapshot(&snapshot, json)?;
    Ok(())
}

fn local_agent_runtime(
    dir: &Path,
    model: Option<String>,
) -> Result<mc_core::agent::MainAgentRuntime> {
    let mut cfg = mc_core::agent::AgentLlmConfig::from_local(dir).with_context(|| {
        "Failed to read the OpenAI API key; set OPENAI_API_KEY or write it to desktop/src-tauri/.env"
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
    let store = mc_core::agent::AgentSessionStore::new(dir);
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
    let store = mc_core::agent::AgentSessionStore::new(&dir);
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
    let store = mc_core::agent::AgentSessionStore::new(&dir);
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
    let store = mc_core::agent::AgentSessionStore::new(dir);
    let mut next = agent
        .continue_from_user_message(snapshot, user_message)
        .await?;
    next.push_trace("saved local agent session");
    store.save_snapshot(&next)?;
    print_agent_snapshot(&next, json)?;
    Ok(next)
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
    let store = mc_core::agent::AgentSessionStore::new(&dir);
    let snapshot = load_agent_snapshot(&store, session_id)?;
    if execution_completed(&snapshot) {
        let mut next = copy_completed_artifact_to_output(snapshot, output)?;
        next.push_trace("saved local agent session");
        store.save_snapshot(&next)?;
        print_agent_snapshot(&next, json)?;
        return Ok(next);
    }
    ensure_session_is_executable(&snapshot)?;
    let agent = deterministic_agent_runtime()?;
    let mut next = agent.advance(snapshot, output).await?;
    next.push_trace("saved local agent session");
    store.save_snapshot(&next)?;
    print_agent_snapshot(&next, json)?;
    Ok(next)
}

fn load_agent_snapshot(
    store: &mc_core::agent::AgentSessionStore,
    session_id: &str,
) -> Result<AgentRunSnapshot> {
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
    let store = mc_core::agent::AgentSessionStore::new(&dir);
    let snapshot = load_agent_snapshot(&store, session_id)?;
    let manifest = exec_smoke_manifest(outcome, reason);
    let mut next = mc_core::agent::continue_after_execution_manifest_result(snapshot, manifest)?;
    next.push_trace("saved local agent session");
    store.save_snapshot(&next)?;
    print_agent_snapshot(&next, json)?;
    Ok(())
}

fn cmd_agent_list(json: bool, all: bool, limit: usize) -> Result<()> {
    let sessions = mc_core::agent::AgentSessionStore::new(data_dir()).list_sessions()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
    } else {
        print_agent_session_list(&sessions, all, limit);
    }
    Ok(())
}

fn cmd_agent_delete(session_id: &str, json: bool) -> Result<()> {
    let deleted = mc_core::agent::AgentSessionStore::new(data_dir()).delete_session(session_id)?;
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

fn print_agent_snapshot(snapshot: &AgentRunSnapshot, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(snapshot)?);
    } else {
        print_agent_snapshot_summary(snapshot);
    }
    Ok(())
}

fn print_agent_snapshot_summary(snapshot: &AgentRunSnapshot) {
    println!("session: {}", snapshot.id);
    println!("status: {}", agent_status_label(&snapshot.status));
    println!("phase: {}", agent_phase_label(&snapshot.phase));

    if let Some(intent) = snapshot.intent.as_ref() {
        println!(
            "intent: {} ({:.2})",
            agent_intent_label(&intent.kind),
            intent.confidence.clamp(0.0, 1.0)
        );
    }

    match snapshot.phase {
        AgentPhase::ConfigureRequirementsApproval => print_requirements_summary(snapshot),
        AgentPhase::ChooseBasePackApproval => print_base_pack_summary(snapshot),
        AgentPhase::ConfirmCustomizationApproval => print_customization_summary(snapshot),
        AgentPhase::ExecutionReady | AgentPhase::Executing | AgentPhase::Verifying => {
            print_execution_summary(snapshot)
        }
        AgentPhase::Completed | AgentPhase::Failed => {
            if let Some(last) = snapshot.messages.last() {
                println!("message: {}", last.text);
            }
        }
        _ => {
            if let Some(plan) = snapshot.plan.as_ref() {
                println!("summary:");
                print_indented(&plan.summary_markdown, 2);
            }
        }
    }

    if let Some(message) = latest_approval_clarification_message(snapshot) {
        println!("message: {message}");
    }

    print_pending_approval_next_steps(snapshot);
}

fn latest_approval_clarification_message(snapshot: &AgentRunSnapshot) -> Option<&str> {
    let was_clarification = snapshot
        .trace
        .iter()
        .rev()
        .find(|event| event.event != "saved local agent session")
        .is_some_and(|event| {
            event
                .event
                .starts_with("approval message needed clarification")
        });
    if !was_clarification {
        return None;
    }
    snapshot
        .messages
        .last()
        .filter(|message| message.kind == AgentMessageKind::Assistant)
        .map(|message| message.text.as_str())
}

fn print_requirements_summary(snapshot: &AgentRunSnapshot) {
    println!("requirements:");
    if let Some(restrictions) = snapshot.restrictions.as_ref() {
        println!(
            "  target: {} / {}",
            restrictions.loader.as_deref().unwrap_or("-"),
            cli_requirement_version_label(restrictions)
        );
        println!("  tags: {}", joined_or_dash(&restrictions.feature_tags));
        if let Some(notes) = restrictions
            .notes
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            println!("  notes: {notes}");
        }
        println!("  revision: {}", restrictions.revision);
    } else {
        println!("  -");
    }
    print_approval_header(snapshot.pending_approval.as_ref());
}

fn cli_requirement_version_label(restrictions: &BuildRestrictions) -> String {
    restrictions
        .minecraft_version
        .as_deref()
        .map(ToOwned::to_owned)
        .or_else(|| restrictions.minecraft_version_requirement.clone())
        .unwrap_or_else(|| "-".to_string())
}

fn print_base_pack_summary(snapshot: &AgentRunSnapshot) {
    print_approval_header(snapshot.pending_approval.as_ref());
    let Some(approval) = snapshot.pending_approval.as_ref() else {
        return;
    };
    println!("base pack options:");
    if approval.options.is_empty() {
        println!("  -");
        return;
    }
    for (idx, option) in approval.options.iter().take(8).enumerate() {
        println!("  {}. {}", idx + 1, option.label);
        if let Some(description) = option
            .description
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            println!(
                "     {}",
                truncate_text(&description.replace('\n', " "), 220)
            );
        }
    }
    if approval.options.len() > 8 {
        println!("  ... {} more", approval.options.len() - 8);
    }
}

fn print_customization_summary(snapshot: &AgentRunSnapshot) {
    print_approval_header(snapshot.pending_approval.as_ref());
    let Some(payload) = recommended_customization_payload(snapshot.pending_approval.as_ref())
    else {
        return;
    };

    if let Some(base) = payload.get("base_pack") {
        println!("base pack: {}", json_string_or(base, "title", "-"));
    }
    if let Some(target) = payload.get("target") {
        println!(
            "target: {} / {}",
            json_string_or(target, "loader", "-"),
            json_string_or(target, "minecraft_version", "-")
        );
    }
    let mods = payload
        .get("extra_mods")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    println!("extra mods: {}", mods.len());
    for (idx, item) in mods.iter().take(10).enumerate() {
        let title = json_string_or(item, "title", "unknown mod");
        let version = item
            .get("resolved_version")
            .and_then(|v| v.get("version_number"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                item.get("source_ref")
                    .and_then(|v| v.get("version_id"))
                    .and_then(|v| v.as_str())
            })
            .or_else(|| item.get("review_version").and_then(|v| v.as_str()))
            .unwrap_or("-");
        let file = item
            .get("source_ref")
            .and_then(|v| v.get("file"))
            .and_then(|v| v.get("filename"))
            .and_then(|v| v.as_str())
            .or_else(|| item.get("review_file").and_then(|v| v.as_str()))
            .unwrap_or("-");
        let review_source = item
            .get("review_source")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let review_reason = item
            .get("review_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let auto_added = item
            .get("auto_added")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        println!("  {}. {} ({version})", idx + 1, title);
        println!(
            "     file: {file} | source: {review_source} | reason: {review_reason}{}",
            if auto_added { " | auto-added" } else { "" }
        );
    }
    if mods.len() > 10 {
        println!("  ... {} more", mods.len() - 10);
    }
}

fn print_execution_summary(snapshot: &AgentRunSnapshot) {
    if let Some(approved) = snapshot.approved_build.as_ref() {
        println!(
            "target: {} / {}",
            json_string_or(&approved.target, "loader", "-"),
            json_string_or(&approved.target, "minecraft_version", "-")
        );
        println!(
            "base pack: {}",
            json_string_or(&approved.base_pack, "title", "-")
        );
        println!("extra mods: {}", approved.extra_mods.len());
    }
    if let Some(execution) = snapshot.execution.as_ref() {
        println!("execution: {:?}", execution.status);
        if let Some(blocked) = execution.blocked.as_ref() {
            println!("blocked: {}", blocked.reason);
        }
    }
}

fn print_approval_header(approval: Option<&ApprovalRequest>) {
    if let Some(approval) = approval {
        println!("approval: {}", approval_kind_label(&approval.kind));
        println!("title: {}", approval.title);
        println!("message: {}", approval.message);
    }
}

fn print_pending_approval_next_steps(snapshot: &AgentRunSnapshot) {
    let Some(approval) = snapshot.pending_approval.as_ref() else {
        if let Some(command) = execution_next_step_command(snapshot) {
            println!("next:");
            println!("  {command}");
        }
        return;
    };

    println!("next:");
    match approval.kind {
        ApprovalKind::ConfigureRequirements => {
            println!(
                "  mc agent continue --session-id {} --message \"Confirm and continue\"",
                snapshot.id
            );
            println!(
                "  mc agent continue --session-id {} --message \"Change it to Fabric 1.20.1 with more exploration and RPG content\"",
                snapshot.id
            );
        }
        ApprovalKind::ChooseBasePack | ApprovalKind::ConfirmScratchFallback => {
            println!(
                "  mc agent continue --session-id {} --message \"Choose the first option\"",
                snapshot.id
            );
            println!(
                "  mc agent continue --session-id {} --message \"Search again with more adventure and exploration, less machinery\"",
                snapshot.id
            );
        }
        ApprovalKind::ConfirmCustomization => {
            println!(
                "  mc agent continue --session-id {} --message \"Confirm this mod plan and continue\"",
                snapshot.id
            );
            println!(
                "  mc agent continue --session-id {} --message \"Remove tech and machinery mods; add more dungeons, structures, exploration, maps, and QoL\"",
                snapshot.id
            );
        }
        ApprovalKind::ReviewDraftPlan => {
            println!(
                "  mc agent continue --session-id {} --message \"Confirm and continue\"",
                snapshot.id
            );
        }
    }
}

fn execution_next_step_command(snapshot: &AgentRunSnapshot) -> Option<String> {
    (snapshot.status == AgentStatus::Running && snapshot.phase == AgentPhase::ExecutionReady).then(
        || {
            format!(
                "mc agent execute --session-id {} --output <path>",
                snapshot.id
            )
        },
    )
}

fn print_agent_session_list(sessions: &[AgentSessionSummary], all: bool, limit: usize) {
    if sessions.is_empty() {
        println!("agent sessions: none");
        return;
    }
    println!("agent sessions:");
    let shown = if all {
        sessions.len()
    } else {
        limit.min(sessions.len())
    };
    for session in sessions.iter().take(shown) {
        let approval = session
            .pending_approval_kind
            .as_ref()
            .map(approval_kind_label)
            .unwrap_or("-");
        println!(
            "- {}  {}  {}  approval={}",
            session.session_id,
            agent_status_label(&session.status),
            agent_phase_label(&session.phase),
            approval
        );
        println!(
            "  prompt: {}",
            truncate_text(&session.user_prompt.replace('\n', " "), 120)
        );
    }
    if shown < sessions.len() {
        println!(
            "... {} more (use --all for full summary or --json for raw data)",
            sessions.len() - shown
        );
    }
}

fn recommended_customization_payload(
    approval: Option<&ApprovalRequest>,
) -> Option<&serde_json::Value> {
    approval?
        .options
        .iter()
        .find(|option| option.id == "confirm:recommended_customization")
        .and_then(|option| option.payload.as_ref())
}

fn json_string_or<'a>(value: &'a serde_json::Value, key: &str, fallback: &'a str) -> &'a str {
    value.get(key).and_then(|v| v.as_str()).unwrap_or(fallback)
}

fn joined_or_dash(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_string()
    } else {
        items.join(", ")
    }
}

fn print_indented(text: &str, spaces: usize) {
    let prefix = " ".repeat(spaces);
    for line in text.lines() {
        println!("{prefix}{line}");
    }
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

fn agent_status_label(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::Running => "running",
        AgentStatus::WaitingForUser => "waiting_for_user",
        AgentStatus::Completed => "completed",
        AgentStatus::Failed => "failed",
    }
}

fn agent_intent_label(kind: &AgentIntentKind) -> &'static str {
    match kind {
        AgentIntentKind::BuildModpack => "build_modpack",
        AgentIntentKind::Unknown => "unknown",
    }
}

fn agent_phase_label(phase: &AgentPhase) -> &'static str {
    match phase {
        AgentPhase::IntentExtraction => "intent_extraction",
        AgentPhase::IntentRouting => "intent_routing",
        AgentPhase::ConfigureRequirementsApproval => "configure_requirements_approval",
        AgentPhase::BasePackSearch => "base_pack_search",
        AgentPhase::BasePackRanking => "base_pack_ranking",
        AgentPhase::ChooseBasePackApproval => "choose_base_pack_approval",
        AgentPhase::CustomizationPlanning => "customization_planning",
        AgentPhase::ConfirmCustomizationApproval => "confirm_customization_approval",
        AgentPhase::ExecutionReady => "execution_ready",
        AgentPhase::Executing => "executing",
        AgentPhase::Verifying => "verifying",
        AgentPhase::Completed => "completed",
        AgentPhase::Failed => "failed",
    }
}

fn approval_kind_label(kind: &ApprovalKind) -> &'static str {
    match kind {
        ApprovalKind::ConfigureRequirements => "configure_requirements",
        ApprovalKind::ChooseBasePack => "choose_base_pack",
        ApprovalKind::ConfirmCustomization => "confirm_customization",
        ApprovalKind::ConfirmScratchFallback => "confirm_scratch_fallback",
        ApprovalKind::ReviewDraftPlan => "review_draft_plan",
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use mc_core::agent::{
        AgentExecutionMetadata, AgentExecutionStatus, ApprovalDecisionSpec, ApprovalOption,
        UserDecisionKind,
    };
    use mc_core::modpack::formats::mrpack::{MrpackDependencies, MrpackIndex};

    fn temp_data_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("mc-agent-cli-{tag}-{}-{nanos}", std::process::id()))
    }

    fn temp_mrpack_path(tag: &str) -> PathBuf {
        temp_data_dir(tag).with_extension("mrpack")
    }

    fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::{Cursor, Write};
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default();
            for (path, bytes) in files {
                zip.start_file(*path, options).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn base_archive_for_cli_execute() -> Vec<u8> {
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

    fn one_response_server(status: u16, content_type: &'static str, body: Vec<u8>) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf);
                let reason = match status {
                    200 => "OK",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "OK",
                };
                let headers = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://{addr}")
    }

    fn approval_route_runtime(decision: serde_json::Value) -> mc_core::agent::MainAgentRuntime {
        let body = openai_response_body(decision.to_string());
        let base_url = one_response_server(200, "application/json", body);
        let mut cfg = mc_core::agent::AgentLlmConfig::new("test-key");
        cfg.base_url = base_url;
        let llm = mc_core::agent::AgentLlmClient::new(cfg).unwrap();
        mc_core::agent::MainAgentRuntime::new(llm)
    }

    fn openai_response_body(output_text: String) -> Vec<u8> {
        serde_json::json!({
            "id": "resp_test",
            "object": "response",
            "created_at": 0,
            "status": "completed",
            "model": "gpt-test",
            "output": [{
                "type": "message",
                "id": "msg_test",
                "status": "completed",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "annotations": [],
                    "text": output_text
                }]
            }],
            "usage": {
                "input_tokens": 1,
                "output_tokens": 1,
                "total_tokens": 2
            },
            "tools": []
        })
        .to_string()
        .into_bytes()
    }

    fn archive_file_payload(url: &str, size: usize) -> serde_json::Value {
        serde_json::json!({
            "url": url,
            "filename": "base.mrpack",
            "sha1": null,
            "sha512": null,
            "size": size,
            "primary": true,
        })
    }

    fn execution_ready_snapshot(
        session_id: &str,
        base_url: &str,
        base_size: usize,
    ) -> AgentRunSnapshot {
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = session_id.to_string();
        run.status = AgentStatus::Running;
        run.phase = AgentPhase::ExecutionReady;
        run.pending_approval = None;
        run.approved_build = Some(mc_core::agent::ApprovedModpackBuild {
            base_pack: serde_json::json!({
                "provider": "modrinth",
                "title": "Base Pack",
            }),
            target: serde_json::json!({
                "minecraft_version": "1.20.1",
                "loader": "fabric",
            }),
            extra_mods: Vec::new(),
            execution_recipe: Some(serde_json::json!({
                "schema_version": 1,
                "kind": "mrpack_from_base_modpack",
                "format": "mrpack",
                "base_pack_ref": {
                    "source_ref": {
                        "archive_file": archive_file_payload(base_url, base_size)
                    }
                },
                "extra_mod_refs": []
            })),
        });
        run.execution = Some(AgentExecutionMetadata {
            status: AgentExecutionStatus::NotStarted,
            manifest: None,
            blocked: None,
        });
        run
    }

    fn customization_approval_snapshot(
        session_id: &str,
        base_url: &str,
        base_size: usize,
    ) -> AgentRunSnapshot {
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = session_id.to_string();
        run.status = AgentStatus::WaitingForUser;
        run.phase = AgentPhase::ConfirmCustomizationApproval;
        run.pending_approval = Some(ApprovalRequest {
            id: "approval-test".to_string(),
            kind: ApprovalKind::ConfirmCustomization,
            title: "Confirm customization plan".to_string(),
            message: "Ready to execute after confirmation".to_string(),
            options: vec![ApprovalOption {
                id: "confirm:recommended_customization".to_string(),
                label: "Confirm recommended plan".to_string(),
                description: None,
                payload: Some(serde_json::json!({
                    "base_pack": {
                        "provider": "modrinth",
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
                                "archive_file": archive_file_payload(base_url, base_size)
                            }
                        },
                        "extra_mod_refs": []
                    }
                })),
            }],
            available_decisions: vec![
                ApprovalDecisionSpec {
                    kind: UserDecisionKind::Approve,
                    label: "Confirm recommended plan".to_string(),
                    requires_selected_option: true,
                    requires_message: false,
                },
                ApprovalDecisionSpec {
                    kind: UserDecisionKind::Revise,
                    label: "Change extra mods".to_string(),
                    requires_selected_option: false,
                    requires_message: true,
                },
            ],
            tools: Vec::new(),
            plan: None,
        });
        run
    }

    #[test]
    fn default_agent_output_path_uses_agent_data_dir() {
        let data_dir =
            std::env::temp_dir().join(format!("mc-agent-cli-test-{}", std::process::id()));

        let output = default_agent_output_path(&data_dir, "session-123");

        assert_eq!(
            output,
            data_dir
                .join("agent")
                .join("artifacts")
                .join("session-123.mrpack")
        );
        assert!(output.is_absolute());
    }

    #[test]
    fn show_missing_session_returns_friendly_error_without_internal_path() {
        let data_dir = temp_data_dir("missing-show");
        let err = cmd_agent_show_with_dir(&data_dir, "missing-session", true)
            .expect_err("missing session should be user-facing");
        let text = err.to_string();

        assert!(text.contains("Session 'missing-session' was not found"));
        assert!(text.contains("mc agent list"));
        assert!(!text.contains("snapshot.json"));
        assert!(!text.contains(data_dir.to_string_lossy().as_ref()));
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn continue_missing_session_returns_friendly_error_without_internal_path() {
        let data_dir = temp_data_dir("missing-continue");
        let runtime = deterministic_agent_runtime().unwrap();

        let err = cmd_agent_continue_with_runtime(
            &data_dir,
            &runtime,
            "missing-session",
            "Continue",
            true,
        )
        .await
        .expect_err("missing session should be user-facing");
        let text = err.to_string();

        assert!(text.contains("Session 'missing-session' was not found"));
        assert!(text.contains("mc agent list"));
        assert!(!text.contains("snapshot.json"));
        assert!(!text.contains(data_dir.to_string_lossy().as_ref()));
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn continue_completed_session_returns_clear_status_error() {
        let data_dir = temp_data_dir("completed-continue");
        let session_id = "completed-continue-session";
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = session_id.to_string();
        run.status = AgentStatus::Completed;
        run.phase = AgentPhase::Completed;
        mc_core::agent::AgentSessionStore::new(&data_dir)
            .save_snapshot(&run)
            .unwrap();
        let runtime = deterministic_agent_runtime().unwrap();

        let err =
            cmd_agent_continue_with_runtime(&data_dir, &runtime, session_id, "Continue", true)
                .await
                .expect_err("completed session should not continue");
        let text = err.to_string();

        assert!(text.contains("This session is completed and cannot be continued."));
        assert!(!text.contains("pending approval"));
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn execution_ready_next_step_points_to_explicit_execute_command() {
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = "session-123".to_string();
        run.status = AgentStatus::Running;
        run.phase = AgentPhase::ExecutionReady;

        let next = execution_next_step_command(&run).expect("execution-ready next step");

        assert_eq!(
            next,
            "mc agent execute --session-id session-123 --output <path>"
        );
    }

    #[tokio::test]
    async fn continue_to_execution_ready_does_not_write_artifact() {
        let data_dir = temp_data_dir("continue-ready");
        let session_id = "continue-ready-session";
        let base_archive = base_archive_for_cli_execute();
        let base_server =
            one_response_server(200, "application/octet-stream", base_archive.clone());
        let run = customization_approval_snapshot(
            session_id,
            &format!("{base_server}/base.mrpack"),
            base_archive.len(),
        );
        let store = mc_core::agent::AgentSessionStore::new(&data_dir);
        store.save_snapshot(&run).unwrap();
        let output = default_agent_output_path(&data_dir, session_id);
        let runtime = approval_route_runtime(serde_json::json!({
            "decision": "approve",
            "selected_option_id": "confirm:recommended_customization",
            "message": null,
            "rationale": "user confirmed"
        }));

        let next =
            cmd_agent_continue_with_runtime(&data_dir, &runtime, session_id, "Confirm plan", true)
                .await
                .expect("continue should reach execution-ready without executing");

        assert_eq!(next.status, AgentStatus::Running);
        assert_eq!(next.phase, AgentPhase::ExecutionReady);
        assert!(!output.exists(), "continue must not write mrpack artifacts");
        let saved = store.load_snapshot(session_id).unwrap();
        assert_eq!(saved.phase, AgentPhase::ExecutionReady);
        assert_eq!(
            execution_next_step_command(&saved).as_deref(),
            Some("mc agent execute --session-id continue-ready-session --output <path>")
        );
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn continue_with_unrelated_approval_message_stays_at_gate_without_artifact() {
        let data_dir = temp_data_dir("continue-unrelated");
        let session_id = "continue-unrelated-session";
        let base_archive = base_archive_for_cli_execute();
        let base_server =
            one_response_server(200, "application/octet-stream", base_archive.clone());
        let run = customization_approval_snapshot(
            session_id,
            &format!("{base_server}/base.mrpack"),
            base_archive.len(),
        );
        let store = mc_core::agent::AgentSessionStore::new(&data_dir);
        store.save_snapshot(&run).unwrap();
        let output = default_agent_output_path(&data_dir, session_id);
        let runtime = approval_route_runtime(serde_json::json!({
            "decision": "needs_clarification",
            "selected_option_id": null,
            "message": null,
            "rationale": "user message is unrelated to the current approval gate"
        }));

        let next = cmd_agent_continue_with_runtime(
            &data_dir,
            &runtime,
            session_id,
            "I want to go to the beach for coffee.",
            true,
        )
        .await
        .expect("continue should save a clarification snapshot instead of failing");

        assert_eq!(next.status, AgentStatus::WaitingForUser);
        assert_eq!(next.phase, AgentPhase::ConfirmCustomizationApproval);
        assert!(next.approved_build.is_none());
        assert!(next.execution.is_none());
        assert!(
            !output.exists(),
            "invalid continue input must not write artifacts"
        );
        let saved = store.load_snapshot(session_id).unwrap();
        assert_eq!(saved.phase, AgentPhase::ConfirmCustomizationApproval);
        assert_eq!(
            saved
                .pending_approval
                .as_ref()
                .map(|approval| &approval.kind),
            Some(&ApprovalKind::ConfirmCustomization)
        );
        let last = saved
            .messages
            .last()
            .expect("clarification should be saved in the snapshot");
        assert_eq!(last.kind, mc_core::agent::AgentMessageKind::Assistant);
        assert!(
            last.text.contains("does not match") && last.text.contains("state was left unchanged"),
            "unexpected clarification: {}",
            last.text
        );
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn clarification_message_is_shown_after_save_trace() {
        let mut snapshot = customization_approval_snapshot(
            "clarification-display-session",
            "https://example.invalid/base.mrpack",
            1024,
        );
        snapshot.push_message(
            AgentMessageKind::Assistant,
            "Choose an available option, describe a change, or cancel.",
        );
        snapshot.push_trace(
            "approval message needed clarification at customization approval: unrelated input",
        );
        snapshot.push_trace("saved local agent session");

        assert_eq!(
            latest_approval_clarification_message(&snapshot),
            Some("Choose an available option, describe a change, or cancel.")
        );
    }

    #[tokio::test]
    async fn execute_writes_artifact_to_requested_output() {
        let data_dir = temp_data_dir("execute-ready");
        let session_id = "execute-ready-session";
        let base_archive = base_archive_for_cli_execute();
        let base_server =
            one_response_server(200, "application/octet-stream", base_archive.clone());
        let run = execution_ready_snapshot(
            session_id,
            &format!("{base_server}/base.mrpack"),
            base_archive.len(),
        );
        let store = mc_core::agent::AgentSessionStore::new(&data_dir);
        store.save_snapshot(&run).unwrap();
        let output = temp_mrpack_path("explicit-output");

        let next = cmd_agent_execute_with_dir(&data_dir, session_id, &output, true)
            .await
            .expect("execute should write requested output");

        assert_eq!(next.status, AgentStatus::Completed);
        assert!(output.exists());
        let file = std::fs::File::open(&output).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert!(archive.by_name("modrinth.index.json").is_ok());
        let default_output = default_agent_output_path(&data_dir, session_id);
        assert!(
            !default_output.exists(),
            "execute must honor --output instead of writing the default path"
        );
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn execute_before_approval_returns_clear_error_without_writing() {
        let data_dir = temp_data_dir("execute-unapproved");
        let session_id = "unapproved-session";
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = session_id.to_string();
        run.status = AgentStatus::WaitingForUser;
        run.phase = AgentPhase::ConfirmCustomizationApproval;
        mc_core::agent::AgentSessionStore::new(&data_dir)
            .save_snapshot(&run)
            .unwrap();
        let output = temp_mrpack_path("unapproved-output");

        let err = cmd_agent_execute_with_dir(&data_dir, session_id, &output, true)
            .await
            .expect_err("execute before approval should fail");

        assert!(
            err.to_string()
                .contains("does not have an approved executable plan"),
            "unexpected error: {err}"
        );
        assert!(!output.exists());
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn execute_completed_session_copies_existing_artifact_to_requested_output() {
        let data_dir = temp_data_dir("execute-completed-copy");
        let session_id = "completed-session";
        let source = temp_mrpack_path("completed-source");
        let output = temp_mrpack_path("completed-new-output");
        let archive = base_archive_for_cli_execute();
        if let Some(parent) = source.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&source, &archive).unwrap();
        let mut run = AgentRunSnapshot::new("make a pack");
        run.id = session_id.to_string();
        run.status = AgentStatus::Completed;
        run.phase = AgentPhase::Completed;
        run.execution = Some(AgentExecutionMetadata {
            status: AgentExecutionStatus::Completed,
            manifest: Some(serde_json::json!({
                "status": "completed",
                "format": "mrpack",
                "output_path": source.to_string_lossy(),
                "output_size": archive.len(),
            })),
            blocked: None,
        });
        mc_core::agent::AgentSessionStore::new(&data_dir)
            .save_snapshot(&run)
            .unwrap();

        let next = cmd_agent_execute_with_dir(&data_dir, session_id, &output, true)
            .await
            .expect("completed execute should copy recorded artifact");

        assert_eq!(next.status, AgentStatus::Completed);
        assert!(output.exists());
        let file = std::fs::File::open(&output).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert!(archive.by_name("modrinth.index.json").is_ok());
        let manifest = next
            .execution
            .as_ref()
            .and_then(|execution| execution.manifest.as_ref())
            .expect("completed manifest should be present");
        assert_eq!(
            manifest.get("output_path").and_then(|v| v.as_str()),
            Some(output.to_string_lossy().as_ref())
        );
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn exec_smoke_manifest_builds_ready_retry_failed_and_blocked_outcomes() {
        let ready = exec_smoke_manifest(AgentExecSmokeOutcome::Ready, None);
        assert_eq!(ready.get("status").and_then(|v| v.as_str()), Some("ready"));

        let retry = exec_smoke_manifest(AgentExecSmokeOutcome::Retry, Some("cdn timed out"));
        assert_eq!(retry.get("status").and_then(|v| v.as_str()), Some("retry"));
        assert_eq!(
            retry.get("error_kind").and_then(|v| v.as_str()),
            Some("network_timeout")
        );

        let failed = exec_smoke_manifest(AgentExecSmokeOutcome::Failed, Some("corrupt archive"));
        assert_eq!(
            failed.get("status").and_then(|v| v.as_str()),
            Some("failed")
        );
        assert_eq!(
            failed.get("reason").and_then(|v| v.as_str()),
            Some("corrupt archive")
        );

        let completed = exec_smoke_manifest(AgentExecSmokeOutcome::Completed, None);
        assert_eq!(
            completed.get("status").and_then(|v| v.as_str()),
            Some("completed")
        );

        let blocked = exec_smoke_manifest(
            AgentExecSmokeOutcome::BlockedRequirements,
            Some("target mismatch"),
        );
        assert_eq!(
            blocked.get("status").and_then(|v| v.as_str()),
            Some("blocked")
        );
        assert_eq!(
            blocked.get("replan_phase").and_then(|v| v.as_str()),
            Some("requirements")
        );
    }
}
