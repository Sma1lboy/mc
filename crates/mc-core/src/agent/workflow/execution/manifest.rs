use super::*;

/// Consume execution phase output and either advance deterministic execution or
/// explicitly return to the right planning/HITL gate.
///
/// The caller must pass metadata produced by deterministic execution code, for
/// example [`compile_mrpack_execution_metadata`]. This function never asks the
/// model to reinterpret the executor result.
pub fn continue_after_execution_manifest_result(
    mut run: AgentRunSnapshot,
    manifest: serde_json::Value,
) -> Result<AgentRunSnapshot> {
    if !matches!(
        run.phase,
        AgentPhase::ExecutionReady | AgentPhase::Executing | AgentPhase::Verifying
    ) {
        return Err(CoreError::other(format!(
            "execution result cannot be applied while run is in phase {:?}",
            run.phase
        )));
    }

    let outcome = classify_execution_outcome(&manifest)?;

    match outcome.kind {
        ExecutionOutcomeKind::Ready => {
            run.enter_phase(AgentPhase::Executing);
            run.tools = vec![export_mrpack_artifact_tool_spec()];
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Ready,
                manifest: Some(manifest),
                blocked: None,
            });
            run.push_message(
                AgentMessageKind::Tool,
                "exec.compile_execution_manifest produced a ready manifest",
            );
            run.push_trace("execution manifest ready; entering executing phase");
            Ok(run)
        }
        ExecutionOutcomeKind::Verifying => {
            run.enter_phase(AgentPhase::Verifying);
            run.tools.clear();
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Running,
                manifest: Some(manifest),
                blocked: None,
            });
            run.push_message(
                AgentMessageKind::Tool,
                "execution artifact written; verifying",
            );
            run.push_trace("execution artifact written; entering verifying phase");
            Ok(run)
        }
        ExecutionOutcomeKind::Completed => {
            let completed_from = run.phase.clone();
            run.complete(AgentPhase::Completed);
            run.tools.clear();
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Completed,
                manifest: Some(manifest),
                blocked: None,
            });
            run.push_message(AgentMessageKind::Tool, "execution completed");
            if completed_from == AgentPhase::Verifying {
                run.push_trace("verification completed; entering completed phase");
            } else {
                run.push_trace("execution completed");
            }
            Ok(run)
        }
        ExecutionOutcomeKind::Blocked => {
            let replan_phase = outcome.replan_phase.clone().ok_or_else(|| {
                CoreError::other("blocked execution outcome missing replan_phase")
            })?;
            let reason = outcome
                .reason
                .clone()
                .unwrap_or_else(|| execution_block_reason(&manifest));
            let blocked = ExecutionBlocked {
                phase: run.phase.clone(),
                reason: reason.clone(),
                replan_phase: Some(replan_phase.clone()),
                details: manifest
                    .get("blocked")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([])),
            };
            let approval = execution_replan_approval(&run, &replan_phase, &manifest, &reason)?;
            // Re-entry keeps the existing plan; pass it through unchanged.
            let plan = run.plan.clone();
            run.request_approval(replan_phase, approval, plan);
            run.tools.clear();
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Blocked,
                manifest: Some(manifest),
                blocked: Some(blocked),
            });
            run.push_message(
                AgentMessageKind::Tool,
                format!("exec.compile_execution_manifest blocked: {reason}"),
            );
            run.push_trace("execution manifest blocked; returned to HITL gate");
            Ok(run)
        }
        ExecutionOutcomeKind::Retry => {
            let reason = outcome
                .reason
                .clone()
                .unwrap_or_else(|| "execution should retry".to_string());
            run.clear_user_interrupt();
            run.tools = vec![export_mrpack_artifact_tool_spec()];
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Retry,
                manifest: Some(manifest),
                blocked: Some(ExecutionBlocked {
                    phase: run.phase.clone(),
                    reason: reason.clone(),
                    replan_phase: outcome.replan_phase.clone(),
                    details: serde_json::Value::Null,
                }),
            });
            run.push_message(
                AgentMessageKind::Tool,
                format!("execution external error is retryable: {reason}"),
            );
            run.push_trace("execution result classified as retryable external error");
            Ok(run)
        }
        ExecutionOutcomeKind::Failed => {
            let reason = outcome
                .reason
                .clone()
                .unwrap_or_else(|| execution_block_reason(&manifest));
            let failed_at = run.phase.clone();
            let details = manifest
                .get("failed")
                .or_else(|| manifest.get("error"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            run.fail(AgentPhase::Failed);
            run.tools.clear();
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Failed,
                manifest: Some(manifest),
                blocked: Some(ExecutionBlocked {
                    phase: failed_at,
                    reason: reason.clone(),
                    replan_phase: outcome.replan_phase.clone(),
                    details,
                }),
            });
            run.push_message(
                AgentMessageKind::Tool,
                format!("execution failed: {reason}"),
            );
            run.push_trace("execution failed with retry gate metadata");
            Ok(run)
        }
    }
}

pub(super) fn classify_execution_outcome(manifest: &serde_json::Value) -> Result<ExecutionOutcome> {
    let status = manifest
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let reason = Some(execution_block_reason(manifest));
    let replan_phase = execution_replan_phase(manifest).ok();

    match status.as_str() {
        "ready" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Ready,
            reason: None,
            replan_phase: None,
        }),
        "verifying" | "verify" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Verifying,
            reason: None,
            replan_phase: None,
        }),
        "completed" | "complete" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Completed,
            reason: None,
            replan_phase: None,
        }),
        "blocked" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Blocked,
            reason,
            replan_phase,
        }),
        "retry" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Retry,
            reason,
            replan_phase,
        }),
        "failed" if manifest_is_retryable_external_error(manifest) => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Retry,
            reason,
            replan_phase,
        }),
        "failed" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Failed,
            reason,
            replan_phase,
        }),
        other => Err(CoreError::other(format!(
            "unsupported execution manifest status: {other:?}"
        ))),
    }
}

pub(super) fn manifest_is_retryable_external_error(manifest: &serde_json::Value) -> bool {
    if manifest
        .get("retryable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return true;
    }
    let kind = manifest
        .get("error_kind")
        .or_else(|| manifest.get("kind"))
        .or_else(|| manifest.get("category"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        kind.as_str(),
        "download_404"
            | "download_timeout"
            | "source_timeout"
            | "source_unavailable"
            | "network"
            | "network_timeout"
    )
}

fn execution_replan_phase(manifest: &serde_json::Value) -> Result<AgentPhase> {
    let raw = manifest
        .get("replan_phase")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("confirm_customization_approval");
    match raw {
        "confirm_customization_approval" | "customization" | "extra_mods" => {
            Ok(AgentPhase::ConfirmCustomizationApproval)
        }
        "choose_base_pack_approval" | "base_pack_search" | "base_pack" => {
            Ok(AgentPhase::ChooseBasePackApproval)
        }
        "configure_requirements_approval" | "requirements" | "target" => {
            Ok(AgentPhase::ConfigureRequirementsApproval)
        }
        other => Err(CoreError::other(format!(
            "unsupported execution replan_phase: {other}"
        ))),
    }
}

fn execution_block_reason(manifest: &serde_json::Value) -> String {
    let blocked = manifest
        .get("blocked")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .take(3)
                .map(|item| {
                    let title = item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("execution item");
                    let reason = item
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("blocked");
                    format!("{title}: {reason}")
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .filter(|s| !s.is_empty());
    blocked.unwrap_or_else(|| {
        manifest
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("execution manifest blocked")
            .to_string()
    })
}

fn execution_replan_approval(
    run: &AgentRunSnapshot,
    replan_phase: &AgentPhase,
    manifest: &serde_json::Value,
    reason: &str,
) -> Result<ApprovalRequest> {
    match replan_phase {
        AgentPhase::ConfirmCustomizationApproval => {
            customization_execution_blocked_approval(run, manifest, reason)
        }
        AgentPhase::ChooseBasePackApproval | AgentPhase::BasePackSearch => {
            base_pack_execution_blocked_approval(run, manifest, reason)
        }
        AgentPhase::ConfigureRequirementsApproval => {
            requirements_execution_blocked_approval(run, reason)
        }
        other => Err(CoreError::other(format!(
            "cannot return execution block to phase {other:?}"
        ))),
    }
}

fn customization_execution_blocked_approval(
    run: &AgentRunSnapshot,
    manifest: &serde_json::Value,
    reason: &str,
) -> Result<ApprovalRequest> {
    let approved = run
        .approved_build
        .as_ref()
        .ok_or_else(|| CoreError::other("execution block has no approved build"))?;
    let base = selected_base_from_approved_build(approved)?;
    let target = target_compatibility_from_payload(&approved.target);
    let (plan, mut approval) = customization_approval(
        &run.user_prompt,
        &base,
        &target,
        approved.base_pack.clone(),
        approved.extra_mods.clone(),
    );
    approval.title = "Execution manifest is blocked; adjust customization".to_string();
    approval.message = format!(
        "The executor was blocked while compiling the mrpack manifest: {reason}. Change the extra mods or return to base-pack selection."
    );
    if let Some(option) = approval
        .options
        .iter_mut()
        .find(|o| o.id == "confirm:recommended_customization")
    {
        if let Some(payload) = option.payload.as_mut().and_then(|v| v.as_object_mut()) {
            payload.insert("execution_blocked".to_string(), manifest.clone());
            if let Some(recipe) = approved.execution_recipe.clone() {
                payload.insert("execution_recipe".to_string(), recipe);
            }
        }
    }
    approval.plan = Some(plan);
    Ok(approval)
}

fn base_pack_execution_blocked_approval(
    run: &AgentRunSnapshot,
    manifest: &serde_json::Value,
    reason: &str,
) -> Result<ApprovalRequest> {
    let approved = run
        .approved_build
        .as_ref()
        .ok_or_else(|| CoreError::other("execution block has no approved build"))?;
    let base = selected_base_from_approved_build(approved)?;
    let provider = provider_slug(base.provider);
    let options = vec![ApprovalOption {
        id: format!("{provider}:{}", base.project_id),
        label: base.title.clone(),
        description: Some(format!(
            "Current base pack is blocked during execution: {reason}"
        )),
        payload: Some({
            let mut payload = approved.base_pack.clone();
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("execution_blocked".to_string(), manifest.clone());
            }
            payload
        }),
    }];
    Ok(ApprovalRequest {
        id: crate::agent::state::new_id("approval"),
        kind: ApprovalKind::ChooseBasePack,
        title: "Execution manifest is blocked; choose a base pack".to_string(),
        message: format!(
            "The executor was blocked while processing the base pack: {reason}. Search for another base pack, or keep the current base pack and retry."
        ),
        options,
        available_decisions: approval_decisions("Keep this base pack", "Search base packs again"),
        tools: vec![update_build_restrictions_tool_spec()],
        plan: Some(ModpackAgentPlan {
            objective: run.user_prompt.clone(),
            summary_markdown: format!("Base-pack execution is blocked: {reason}"),
            risks: vec![
                "Continuing with the current base pack may hit the same execution block again."
                    .to_string(),
            ],
            planned_actions: vec![PlannedAction {
                id: "replan-base-pack".to_string(),
                label: "User revises base pack after execution block".to_string(),
                tool: "approval_gate".to_string(),
                args: serde_json::json!({ "kind": "choose_base_pack", "execution_blocked": true }),
                requires_approval: true,
            }],
            migration_notes: vec![],
        }),
    })
}

fn requirements_execution_blocked_approval(
    run: &AgentRunSnapshot,
    reason: &str,
) -> Result<ApprovalRequest> {
    let output = run
        .restrictions
        .clone()
        .unwrap_or_default()
        .as_update_output(vec![format!("Execution manifest is blocked: {reason}")]);
    let mut approval = requirements_approval(&run.user_prompt, &output);
    approval.title = "Execution manifest is blocked; adjust requirements".to_string();
    approval.message = format!(
        "The executor cannot continue with the current version/loader/requirements: {reason}. Change the requirements before continuing."
    );
    Ok(approval)
}
