use super::*;

pub(super) async fn run_export_mrpack_artifact_tool(
    mut run: AgentRunSnapshot,
    args: serde_json::Value,
) -> Result<AgentRunSnapshot> {
    let output_path = args
        .get("output_path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| CoreError::other("export_mrpack_artifact requires output_path"))?;
    let mut retry_count = 0;
    let mut dispatch_iteration = 0;
    loop {
        if run.status != AgentStatus::Running {
            return Ok(run);
        }
        if !matches!(
            run.phase,
            AgentPhase::ExecutionReady | AgentPhase::Executing | AgentPhase::Verifying
        ) {
            return Ok(run);
        }
        match run.execution.as_ref().map(|execution| &execution.status) {
            Some(AgentExecutionStatus::Completed) => return Ok(run),
            Some(AgentExecutionStatus::Blocked | AgentExecutionStatus::Failed) => {
                return Ok(run);
            }
            _ => {}
        }

        let approved = run
            .approved_build
            .clone()
            .ok_or_else(|| CoreError::other("execution requires an approved build"))?;
        if run.phase == AgentPhase::Verifying {
            let started = Instant::now();
            let manifest = match verify_written_mrpack(&output_path, &approved) {
                Ok(()) => execution_verification_completed_manifest(&run, &output_path),
                Err(err) => execution_verification_failed_manifest(&err.to_string()),
            };
            let next = continue_execution_manifest_with_trace(
                run,
                manifest,
                &output_path,
                "deterministic verification dispatched",
                "verify_mrpack_artifact",
                dispatch_iteration,
                started,
            )?;
            dispatch_iteration += 1;
            retry_count = 0;
            run = next;
            continue;
        }
        let started = Instant::now();
        let manifest = execute_mrpack_build_to_path(&approved, &output_path).await?;
        let next = continue_execution_manifest_with_trace(
            run,
            manifest,
            &output_path,
            "deterministic execution tool dispatched",
            EXPORT_MRPACK_ARTIFACT_TOOL,
            dispatch_iteration,
            started,
        )?;
        dispatch_iteration += 1;

        if next.execution.as_ref().map(|execution| &execution.status)
            == Some(&AgentExecutionStatus::Retry)
        {
            retry_count += 1;
            let reason = execution_retry_reason(&next);
            if retry_count >= EXECUTION_MAX_RETRIES {
                let manifest = execution_retry_exhausted_manifest(&reason, retry_count);
                run = continue_after_execution_manifest_result(next, manifest)?;
                continue;
            }

            run = next;
            let backoff = execution_retry_backoff(retry_count, EXECUTION_RETRY_BACKOFF_BASE);
            if !backoff.is_zero() {
                tokio::time::sleep(backoff).await;
            }
        } else {
            retry_count = 0;
            run = next;
        }
    }
}

fn continue_execution_manifest_with_trace(
    run: AgentRunSnapshot,
    manifest: serde_json::Value,
    output_path: &Path,
    event: &str,
    tool: &str,
    iteration: u32,
    started: Instant,
) -> Result<AgentRunSnapshot> {
    let status = manifest_status(&manifest);
    let input = serde_json::json!({
        "output_path": output_path.to_string_lossy().to_string(),
    });
    let output = serde_json::json!({ "manifest": manifest.clone() });
    let dispatch_phase = run.phase.clone();
    let mut next = continue_after_execution_manifest_result(run, manifest)?;
    next.push_tool_trace(AgentToolTrace {
        event: event.into(),
        stage: dispatch_phase,
        iteration,
        tool: tool.into(),
        input,
        output,
        duration_ms: started.elapsed().as_millis(),
        status,
    });
    Ok(next)
}

fn manifest_status(manifest: &serde_json::Value) -> String {
    manifest
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string()
}

pub(in crate::agent::workflow) fn clarify_pending_approval_input(
    mut run: AgentRunSnapshot,
    approval: &ApprovalRequest,
    user_message: &str,
    reason: &str,
) -> AgentRunSnapshot {
    let message = approval_clarification_message(approval);
    run.status = AgentStatus::WaitingForUser;
    run.push_message(AgentMessageKind::User, user_message.trim());
    run.push_message(AgentMessageKind::Assistant, message.clone());
    run.push_stream_event(
        AgentStreamEventKind::ClarificationNeeded,
        format!(
            "approval message needed clarification at {}: {}",
            approval_kind_context_label(&approval.kind),
            reason.trim()
        ),
        Some(run.phase.clone()),
        serde_json::json!({
            "approval_id": approval.id,
            "approval_kind": approval.kind,
            "user_message": user_message.trim(),
            "reason": reason.trim(),
            "message": message,
        }),
    );
    run
}

fn approval_clarification_message(approval: &ApprovalRequest) -> String {
    format!(
        "That reply does not match the current {} approval gate. The session state was left unchanged. Choose or confirm an available option, describe the change you want, or cancel.",
        approval_kind_context_label(&approval.kind)
    )
}

fn approval_kind_context_label(kind: &ApprovalKind) -> &'static str {
    match kind {
        ApprovalKind::ConfigureRequirements => "requirements confirmation",
        ApprovalKind::ChooseBasePack => "base pack selection",
        ApprovalKind::ConfirmCustomization => "customization confirmation",
        ApprovalKind::ConfirmScratchFallback => "scratch build confirmation",
        ApprovalKind::ReviewDraftPlan => "draft plan review",
    }
}

fn execution_retry_reason(run: &AgentRunSnapshot) -> String {
    run.execution
        .as_ref()
        .and_then(|execution| execution.blocked.as_ref())
        .map(|blocked| blocked.reason.clone())
        .or_else(|| {
            run.execution
                .as_ref()
                .and_then(|execution| execution.manifest.as_ref())
                .and_then(|manifest| manifest.get("reason"))
                .and_then(|reason| reason.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "execution should retry".to_string())
}

pub(super) fn execution_retry_exhausted_manifest(reason: &str, attempts: u32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "status": "failed",
        "format": "mrpack",
        "reason": format!("execution exceeded max retries: {reason}"),
        "error_kind": "retry_exhausted",
        "retryable": false,
        "attempts": attempts,
    })
}

fn execution_verification_completed_manifest(
    run: &AgentRunSnapshot,
    output_path: &Path,
) -> serde_json::Value {
    let mut manifest = run
        .execution
        .as_ref()
        .and_then(|execution| execution.manifest.clone())
        .unwrap_or_else(|| serde_json::json!({ "schema_version": 1, "format": "mrpack" }));
    set_manifest_field(&mut manifest, "status", serde_json::json!("completed"));
    set_manifest_field(&mut manifest, "verified", serde_json::json!(true));
    set_manifest_field(
        &mut manifest,
        "output_path",
        serde_json::json!(output_path.to_string_lossy().to_string()),
    );
    manifest
}

fn set_manifest_field(value: &mut serde_json::Value, key: &str, next: serde_json::Value) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert(key.to_string(), next);
    }
}

fn execution_verification_failed_manifest(reason: &str) -> serde_json::Value {
    let reason = if reason.starts_with("mrpack verification failed:") {
        reason.to_string()
    } else {
        format!("mrpack verification failed: {reason}")
    };
    serde_json::json!({
        "schema_version": 1,
        "status": "failed",
        "format": "mrpack",
        "reason": reason,
        "error_kind": "verification_failed",
        "retryable": false,
    })
}

fn execution_retry_backoff(attempt: u32, base: Duration) -> Duration {
    let multiplier = 1_u32 << attempt.saturating_sub(1).min(8);
    let delay = base.saturating_mul(multiplier);
    delay.min(EXECUTION_RETRY_BACKOFF_MAX)
}

pub(super) fn export_mrpack_artifact_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: EXPORT_MRPACK_ARTIFACT_TOOL.to_string(),
        description:
            "Export the approved modpack build as a .mrpack file at the requested output path."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["output_path"],
            "properties": {
                "output_path": {
                    "type": "string",
                    "description": "Destination .mrpack path to write."
                }
            }
        }),
        output_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["manifest"],
            "properties": {
                "manifest": {
                    "type": "object",
                    "description": "Deterministic execution manifest returned by the mrpack executor."
                }
            }
        }),
    }
}
