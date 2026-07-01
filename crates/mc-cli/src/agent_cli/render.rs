use super::*;

pub(super) fn print_agent_snapshot(snapshot: &AgentRunSnapshot, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(snapshot)?);
    } else {
        print_agent_snapshot_summary(snapshot);
    }
    Ok(())
}

pub(super) fn print_agent_snapshot_summary(snapshot: &AgentRunSnapshot) {
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

    if snapshot.pending_approval.is_none() {
        print_pending_input_interrupt(snapshot);
    }

    if let Some(message) = latest_approval_clarification_message(snapshot) {
        println!("message: {message}");
    }

    print_pending_approval_next_steps(snapshot);
}

pub(super) fn latest_approval_clarification_message(snapshot: &AgentRunSnapshot) -> Option<&str> {
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

pub(super) fn print_requirements_summary(snapshot: &AgentRunSnapshot) {
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

pub(super) fn cli_requirement_version_label(restrictions: &BuildRestrictions) -> String {
    restrictions
        .minecraft_version
        .as_deref()
        .map(ToOwned::to_owned)
        .or_else(|| restrictions.minecraft_version_requirement.clone())
        .unwrap_or_else(|| "-".to_string())
}

pub(super) fn print_base_pack_summary(snapshot: &AgentRunSnapshot) {
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

pub(super) fn print_customization_summary(snapshot: &AgentRunSnapshot) {
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
    let unresolved = customization_unresolved_request_lines(payload);
    if !unresolved.is_empty() {
        println!("unresolved requests:");
        for line in unresolved {
            println!("  - {line}");
        }
    }
}

pub(super) fn customization_unresolved_request_lines(payload: &serde_json::Value) -> Vec<String> {
    payload
        .get("validation")
        .and_then(|v| v.get("unresolved_goals"))
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let label = item
                .get("label")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())?;
            let diagnosis = item
                .get("diagnosis")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("No compatible candidate was selected.");
            let next_step = item
                .get("next_step")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            Some(match next_step {
                Some(next_step) => format!("{label}: {diagnosis} Next: {next_step}"),
                None => format!("{label}: {diagnosis}"),
            })
        })
        .collect()
}

pub(super) fn print_execution_summary(snapshot: &AgentRunSnapshot) {
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

pub(super) fn print_approval_header(approval: Option<&ApprovalRequest>) {
    if let Some(approval) = approval {
        println!("approval: {}", approval_kind_label(&approval.kind));
        println!("title: {}", approval.title);
        println!("message: {}", approval.message);
    }
}

pub(super) fn print_pending_input_interrupt(snapshot: &AgentRunSnapshot) {
    let Some(interrupt) = snapshot
        .pending_interrupt
        .as_ref()
        .filter(|interrupt| interrupt.kind == AgentInterruptKind::UserInput)
    else {
        return;
    };

    if let Some(input_kind) = interrupt.input_kind.as_ref() {
        println!("input: {}", input_kind_label(input_kind));
    }
    println!("title: {}", interrupt.title);
    println!("message: {}", interrupt.message);
    if !interrupt.options.is_empty() {
        println!("options:");
        for (idx, option) in interrupt.options.iter().take(8).enumerate() {
            println!("  {}. {}", idx + 1, option.label);
            if let Some(description) = option
                .description
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            {
                println!("     {}", truncate_text(description, 220));
            }
        }
        if interrupt.options.len() > 8 {
            println!("  ... {} more", interrupt.options.len() - 8);
        }
    }
}

pub(super) fn print_pending_approval_next_steps(snapshot: &AgentRunSnapshot) {
    let next_steps = pending_approval_next_step_lines(snapshot);
    if !next_steps.is_empty() {
        println!("next:");
        for line in next_steps {
            println!("  {line}");
        }
    }
}

pub(super) fn pending_approval_next_step_lines(snapshot: &AgentRunSnapshot) -> Vec<String> {
    let Some(approval) = snapshot.pending_approval.as_ref() else {
        if let Some(lines) = pending_input_next_step_lines(snapshot) {
            return lines;
        }
        return execution_next_step_command(snapshot).into_iter().collect();
    };

    match approval.kind {
        ApprovalKind::ConfigureRequirements => vec![
            format!(
                "mc agent continue --session-id {} --message \"Confirm and continue\"",
                snapshot.id
            ),
            format!(
                "mc agent continue --session-id {} --message \"Change it to Fabric 1.20.1 with more exploration and RPG content\"",
                snapshot.id
            ),
        ],
        ApprovalKind::ChooseBasePack | ApprovalKind::ConfirmScratchFallback => vec![
            format!(
                "mc agent continue --session-id {} --message \"Choose the first option\"",
                snapshot.id
            ),
            format!(
                "mc agent continue --session-id {} --message \"Search again with more adventure and exploration, less machinery\"",
                snapshot.id
            ),
        ],
        ApprovalKind::ConfirmCustomization => customization_next_step_lines(snapshot, approval),
        ApprovalKind::ReviewDraftPlan => vec![format!(
            "mc agent continue --session-id {} --message \"Confirm and continue\"",
            snapshot.id
        )],
    }
}

pub(super) fn pending_input_next_step_lines(snapshot: &AgentRunSnapshot) -> Option<Vec<String>> {
    let interrupt = snapshot
        .pending_interrupt
        .as_ref()
        .filter(|interrupt| interrupt.kind == AgentInterruptKind::UserInput)?;
    match interrupt.input_kind.as_ref()? {
        AgentInputKind::SelectMinecraftVersion => {
            let example = interrupt
                .options
                .first()
                .map(|option| option.id.as_str())
                .unwrap_or("<version>");
            Some(vec![format!(
                "mc agent continue --session-id {} --message \"{}\"",
                snapshot.id, example
            )])
        }
    }
}

pub(super) fn customization_next_step_lines(
    snapshot: &AgentRunSnapshot,
    approval: &ApprovalRequest,
) -> Vec<String> {
    let mut lines = Vec::new();
    if approval
        .options
        .iter()
        .any(|option| option.id == "confirm:recommended_customization")
    {
        lines.push(format!(
            "mc agent continue --session-id {} --message \"Confirm this mod plan and continue\"",
            snapshot.id
        ));
    }
    if approval
        .available_decisions
        .iter()
        .any(|decision| decision.kind == UserDecisionKind::Revise)
    {
        lines.push(format!(
            "mc agent continue --session-id {} --message \"Remove tech and machinery mods; add more dungeons, structures, exploration, maps, and QoL\"",
            snapshot.id
        ));
    }
    if approval
        .options
        .iter()
        .any(|option| option.id == "back:choose_base_pack")
    {
        lines.push(format!(
            "mc agent continue --session-id {} --message \"Back to base-pack selection\"",
            snapshot.id
        ));
    }
    lines
}

pub(super) fn execution_next_step_command(snapshot: &AgentRunSnapshot) -> Option<String> {
    (snapshot.status == AgentStatus::Running && snapshot.phase == AgentPhase::ExecutionReady).then(
        || {
            format!(
                "mc agent export --session-id {} --output <path>",
                snapshot.id
            )
        },
    )
}

pub(super) fn print_agent_session_list(sessions: &[AgentSessionSummary], all: bool, limit: usize) {
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

pub(super) fn recommended_customization_payload(
    approval: Option<&ApprovalRequest>,
) -> Option<&serde_json::Value> {
    approval?
        .options
        .iter()
        .find(|option| option.id == "confirm:recommended_customization")
        .and_then(|option| option.payload.as_ref())
}

pub(super) fn json_string_or<'a>(
    value: &'a serde_json::Value,
    key: &str,
    fallback: &'a str,
) -> &'a str {
    value.get(key).and_then(|v| v.as_str()).unwrap_or(fallback)
}

pub(super) fn joined_or_dash(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_string()
    } else {
        items.join(", ")
    }
}

pub(super) fn print_indented(text: &str, spaces: usize) {
    let prefix = " ".repeat(spaces);
    for line in text.lines() {
        println!("{prefix}{line}");
    }
}

pub(super) fn truncate_text(text: &str, max_chars: usize) -> String {
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

pub(super) fn agent_status_label(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::Running => "running",
        AgentStatus::WaitingForUser => "waiting_for_user",
        AgentStatus::Completed => "completed",
        AgentStatus::Failed => "failed",
    }
}

pub(super) fn agent_intent_label(kind: &AgentIntentKind) -> &'static str {
    match kind {
        AgentIntentKind::BuildModpack => "build_modpack",
        AgentIntentKind::Unknown => "unknown",
    }
}

pub(super) fn agent_phase_label(phase: &AgentPhase) -> &'static str {
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

pub(super) fn approval_kind_label(kind: &ApprovalKind) -> &'static str {
    match kind {
        ApprovalKind::ConfigureRequirements => "configure_requirements",
        ApprovalKind::ChooseBasePack => "choose_base_pack",
        ApprovalKind::ConfirmCustomization => "confirm_customization",
        ApprovalKind::ConfirmScratchFallback => "confirm_scratch_fallback",
        ApprovalKind::ReviewDraftPlan => "review_draft_plan",
    }
}

pub(super) fn input_kind_label(kind: &AgentInputKind) -> &'static str {
    match kind {
        AgentInputKind::SelectMinecraftVersion => "select_minecraft_version",
    }
}
