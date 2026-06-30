use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};

use mc_core::agent::{
    AgentIntentKind, AgentPhase, AgentRunSnapshot, AgentSessionSummary, AgentStatus, ApprovalKind,
    ApprovalRequest, BuildRestrictions,
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
        for path in mc_core::agent::OpenAiConfig::local_env_paths(&dir) {
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
    let mut cfg = mc_core::agent::OpenAiConfig::from_local(dir).with_context(|| {
        "读取 OpenAI API key 失败;设置 OPENAI_API_KEY 或写入 desktop/src-tauri/.env"
    })?;
    if let Some(model) = model.filter(|s| !s.trim().is_empty()) {
        cfg.model = model;
    }

    let openai = mc_core::agent::OpenAiClient::new(cfg)?;
    Ok(mc_core::agent::MainAgentRuntime::new(openai))
}

fn cmd_agent_show(session_id: &str, json: bool) -> Result<()> {
    let snapshot = mc_core::agent::AgentSessionStore::new(data_dir()).load_snapshot(session_id)?;
    print_agent_snapshot(&snapshot, json)?;
    Ok(())
}

async fn cmd_agent_continue(session_id: &str, message: &str, json: bool) -> Result<()> {
    let user_message = message.trim();
    if user_message.is_empty() {
        anyhow::bail!("--message must not be empty");
    }

    let dir = data_dir();
    let store = mc_core::agent::AgentSessionStore::new(&dir);
    let snapshot = store.load_snapshot(session_id)?;
    let agent = local_agent_runtime(&dir, None)?;
    let mut next = agent
        .continue_from_user_message(snapshot, user_message)
        .await?;
    next.push_trace("saved local agent session");
    store.save_snapshot(&next)?;
    print_agent_snapshot(&next, json)?;
    Ok(())
}

async fn cmd_agent_execute(session_id: &str, output: &Path, json: bool) -> Result<()> {
    let dir = data_dir();
    let store = mc_core::agent::AgentSessionStore::new(&dir);
    let snapshot = store.load_snapshot(session_id)?;
    let approved = snapshot
        .approved_build
        .as_ref()
        .context("agent session has no approved build to execute")?;
    let manifest = mc_core::agent::execute_mrpack_build_to_path(approved, output).await?;
    let mut next = mc_core::agent::continue_after_execution_manifest_result(snapshot, manifest)?;
    next.push_trace("saved local agent session");
    store.save_snapshot(&next)?;
    print_agent_snapshot(&next, json)?;
    Ok(())
}

fn cmd_agent_exec_smoke(
    session_id: &str,
    outcome: AgentExecSmokeOutcome,
    reason: Option<&str>,
    json: bool,
) -> Result<()> {
    let dir = data_dir();
    let store = mc_core::agent::AgentSessionStore::new(&dir);
    let snapshot = store.load_snapshot(session_id)?;
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

    print_pending_approval_next_steps(snapshot);
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
        if matches!(snapshot.phase, AgentPhase::ExecutionReady) {
            println!(
                "next: mc agent execute --session-id {} --output ./dist/{}.mrpack",
                snapshot.id, snapshot.id
            );
        }
        return;
    };

    println!("next:");
    match approval.kind {
        ApprovalKind::ConfigureRequirements => {
            println!(
                "  mc agent continue --session-id {} --message \"确认继续\"",
                snapshot.id
            );
            println!(
                "  mc agent continue --session-id {} --message \"改成 Fabric 1.20.1，更偏探索和 RPG\"",
                snapshot.id
            );
        }
        ApprovalKind::ChooseBasePack | ApprovalKind::ConfirmScratchFallback => {
            println!(
                "  mc agent continue --session-id {} --message \"就选第一个\"",
                snapshot.id
            );
            println!(
                "  mc agent continue --session-id {} --message \"换一批，更偏冒险探索，少一点纯机械\"",
                snapshot.id
            );
        }
        ApprovalKind::ConfirmCustomization => {
            println!(
                "  mc agent continue --session-id {} --message \"确认这个 mod 方案，继续\"",
                snapshot.id
            );
            println!(
                "  mc agent continue --session-id {} --message \"去掉偏科技和机械的 mod，多加地牢、结构、探索、地图和 QoL\"",
                snapshot.id
            );
        }
        ApprovalKind::ReviewDraftPlan => {
            println!(
                "  mc agent continue --session-id {} --message \"确认继续\"",
                snapshot.id
            );
        }
    }
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
