//! Main-agent workflow entry points and the modpack-build subworkflow.
//!
//! The top-level agent should route user intents into focused subworkflows. This
//! file currently implements one such capability: `ModpackBuildWorkflow`.
//! Its planning phase may call the LLM and pause at HITL gates; execution
//! remains deterministic daemon/core work and is intentionally outside this
//! planning subworkflow.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::{CoreError, Result};
use crate::modpack::export::modrinth::host_in_whitelist;
use crate::modplatform::dependency::{resolve_dependencies, ModRef};
use crate::modplatform::provider::ProviderRegistry;
use crate::modplatform::{
    ProjectSideSupport, ProviderId, ResolvedFile, ResourceKind, SearchHit, SearchQuery, SortMethod,
    VersionFile,
};

use super::llm::AgentLlmClient;
use super::state::{
    AgentExecutionMetadata, AgentExecutionStatus, AgentIntent, AgentIntentKind, AgentMessageKind,
    AgentPhase, AgentRunSnapshot, AgentStatus, AgentToolSpec, AgentToolTrace, AgentWorkflowKind,
    ApprovalDecisionSpec, ApprovalKind, ApprovalOption, ApprovalRequest, ApprovedModpackBuild,
    BuildRestrictionChange, BuildRestrictionChangeSource, BuildRestrictionPatch, BuildRestrictions,
    ExecutionBlocked, Goal, GoalKind, GoalQuery, GoalStatus, ModPlanState, ModProvenance,
    ModpackAgentPlan, PlanArtifact, PlanReplanRequest, PlannedAction, ResolvedMod,
    TargetCompatibility, UpdateBuildRestrictionsInput, UpdateBuildRestrictionsOutput, UserDecision,
    UserDecisionKind,
};

mod approvals;
mod artifacts;
mod base_modlist;
mod base_search;
mod customization;
mod execution;
mod llm_io;
mod requirements;

use artifacts::{
    attach_base_pack_resolution, candidate_option, customization_approval,
    customization_approval_with_validation, json_str_or, mod_payload,
    mrpack_file_payload_with_filename, project_url, provider_label, provider_slug,
    safe_provider_filename, scratch_base_pack_option, scratch_base_pack_payload,
    scratch_build_plan, selection_plan, source_ref_payload, version_file_payload,
    version_file_with_project_side,
};

use approvals::{
    approval_decisions, approved_build_from_payload, base_pack_selection_approval,
    missing_restriction_fields, requirement_label, requirement_summary_message,
    requirements_approval, requirements_plan, restrictions_from_requirement_payload,
};
#[cfg(test)]
use artifacts::{mrpack_file_payload, resolved_mod_payload};
use base_modlist::{fetch_base_modlist_cache, mod_ref_payloads};
#[cfg(test)]
use base_search::{base_search_has_acceptable_count, next_base_search_mode};
use base_search::{
    block_base_pack_planning, continue_after_base_pack_choice, continue_after_base_pack_feedback,
    continue_to_base_pack_search,
};
#[cfg(test)]
use customization::customization_blockers;
#[cfg(test)]
use customization::remove_existing_mod_payloads;
#[cfg(test)]
use customization::{
    append_dependency_resolution, apply_mod_plan_step, baseline_mod_refs,
    fallback_mod_search_queries, initialize_mod_plan_state, merge_feedback_into_mod_plan,
    prefilter_mod_candidates, unresolved_mod_plan_goals,
};
use customization::{
    block_customization_planning, continue_after_customization_confirmation,
    continue_after_customization_feedback, infer_base_pack_compatibility,
    run_customization_planning_loop,
};
#[cfg(test)]
use llm_io::parse_approval_decision_response;
#[cfg(test)]
use llm_io::ApprovalDecisionOutputKind;
#[cfg(test)]
use llm_io::CustomizationCritiqueOutput;
#[cfg(test)]
use llm_io::CustomizationCritiqueVerdictOutput;
#[cfg(test)]
use llm_io::ModSelection;
use llm_io::{
    dedupe_queries, normalize_mod_search_query, update_build_restrictions_tool_spec,
    AgentIntentOutput, ApprovalRoute, ApprovalRouteOutput, BaseCoverageOutput, ModPlanControl,
    ModPlanStep, SearchQueryOutput,
};
#[cfg(test)]
use llm_io::{parse_intent_response, parse_mod_query_response, search_queries};
#[cfg(test)]
use requirements::{
    apply_requirements_replan, invalidation_rule_for_changed_field,
    normalize_restriction_update_input, parse_restriction_update_response,
    restriction_update_request_payload, target_phase_for_changed_field,
    validate_restriction_update_retry, ALL_CHANGED_FIELDS,
};
use requirements::{
    changed_restriction_field, continue_after_requirements_confirmation,
    continue_after_requirements_feedback, generate_restriction_update, invalidate_downstream,
    maybe_replan_requirements_from_feedback, update_build_restrictions,
};

#[cfg(test)]
use base_modlist::{
    base_modlist_cache_from_archive_bytes, ensure_base_archive_size, parse_base_modlist,
};

use execution::verify_written_mrpack;
pub use execution::{
    build_mrpack_from_base_archive_bytes, compile_mrpack_execution_metadata,
    continue_after_execution_manifest_result, execute_mrpack_build_to_path, MrpackExecutionBuild,
    MrpackOverrideFile,
};

const UPDATE_BUILD_RESTRICTIONS_TOOL: &str = "update_build_restrictions";
const BUILD_MRPACK_ARTIFACT_TOOL: &str = "build_mrpack_artifact";
const BASE_SEARCH_MAX_ITERATIONS: u32 = 4;
const BASE_SEARCH_MIN_CANDIDATES: usize = 3;
const BASE_SEARCH_MAX_CANDIDATES: usize = 12;
const BASE_SEARCH_APPROVAL_LIMIT: usize = 6;
const MOD_PLAN_ROUND_CAP: u32 = 6;
const BASE_ARCHIVE_FETCH_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_BASE_ARCHIVE_BYTES: usize = 128 * 1024 * 1024;
const MAX_BASE_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const EXECUTION_MAX_RETRIES: u32 = 3;
const EXECUTION_RETRY_BACKOFF_BASE: Duration = Duration::from_millis(500);
const EXECUTION_RETRY_BACKOFF_MAX: Duration = Duration::from_secs(4);

const MAIN_AGENT_SYSTEM_PROMPT: &str = r#"You are the local AI agent for a Minecraft launcher.
Your job is to turn user requests into safe daemon-owned workflows, not to perform game file writes directly.

Current capabilities and boundaries:
- Route each user request before taking workflow action.
- For modpack creation, prefer recommending an existing base modpack, then customize it with compatible existing mods.
- If no base pack is suitable, the workflow may offer a human-approved scratch fallback that starts from an empty base set and uses deterministic dependency resolution.
- Search, import, install, export, and file writes are deterministic daemon tools; do not invent platform results, files, versions, or installation state.
- Stop at human approval gates before choosing a base pack, approving customization, or starting any write/install/export execution.
- General mod/modpack how-to questions are not handled by this local workflow yet; route unsupported requests to unknown.
- Keep outputs machine-readable when a subtask asks for a schema."#;

const INTENT_ROUTING_PROMPT: &str = r#"Classify the user's request into exactly one intent:
- build_modpack: create, customize, recommend, or generate a new modpack from requirements.
- unknown: anything else, including crash diagnosis, instance management, export/share requests, general how-to questions, ambiguous requests, unsupported requests, or unrelated requests.

Return an object matching the provided schema."#;

const APPROVAL_DECISION_ROUTING_PROMPT: &str = r#"Convert the user's latest message into a decision for the current pending approval gate.
Use only the current approval's kind, available decisions, options, and tool schemas.
Return approve only when the user clearly accepts one available option. If the user refers to an option by ordinal words like first/second/third, map it to the matching option id.
Return revise when the user asks to change, replace, search again, add requirements, remove requirements, or otherwise modify the current proposal.
Return cancel when the user clearly asks to stop or cancel.
Return needs_clarification when the message is ambiguous, asks an unrelated question, or cannot be mapped to the current approval gate.
Do not skip future workflow gates. If the user mentions future-stage requirements, preserve them in revise feedback instead of jumping ahead.
Return an object matching the provided schema."#;

const REQUIREMENT_NORMALIZATION_PROMPT: &str = r#"Generate arguments for the update_build_restrictions tool.
Do not search for modpacks or mods.
Do not choose default values for missing fields.
Only set minecraft_version when the user explicitly gives a concrete Minecraft version such as 1.20.1.
Set minecraft_version_requirement to the raw user-facing version requirement when present, including concrete versions and ranges such as 1.20.x, <=1.19.x, or 1.20.1/1.20.4.
Only set loader to fabric, forge, neoforge, or quilt when the user explicitly asks for that loader.
Use null when the loader is absent or ambiguous.
Feature tags should be short search/use-case tags from the user's request, not full sentences.
The patch represents the full desired BuildRestrictions state after applying the latest user message, not only a delta.
Return an object matching the provided tool input schema."#;

const REQUIREMENT_NORMALIZATION_RETRY_PROMPT: &str = r#"The previous response violated the schema contract.
Return exactly one JSON object matching the update_build_restrictions tool input schema.
Do not return multiple objects, markdown fences, explanations, or copied previous output."#;

const SEARCH_QUERY_PROMPT: &str = r#"You are planning the base-pack search step for a Minecraft modpack build workflow.
Return short English search queries for finding an existing base modpack.
Prefer canonical project/mod names or well-known ecosystem terms implied by the user's request over broad category phrases.
Include specific requirement keywords that are likely to appear in project titles or descriptions.
Each query must be a concise platform search string, not a sentence.
Use separate short queries instead of one long query that combines every requirement.
Across the query set, cover every major user-requested feature instead of focusing on only one theme.
Do not include generic words like "Minecraft", "modpack", "base pack", or "pack"; the search tool already filters for modpacks.
Prefer mature base modpacks. The scratch fallback is a later human-approved option, not a search query target.
Return an object matching the provided schema."#;

const MOD_PLAN_STEP_PROMPT: &str = r#"You are planning a compatible Minecraft mod set.
The runtime has already searched provider candidates and filtered obvious duplicates.
Select only project_id values from the provided candidate pool. Do not invent URLs, version ids, filenames, hashes, or environment metadata.
Use selections to cover open goals, removals to drop unwanted current additions, next_queries for the next deterministic search round, and control=done only when the current set is coherent enough for the final human approval gate.
When candidate_pool is empty or insufficient, next_queries must be short provider search terms: canonical mod/project names or 2-5 keyword phrases.
Do not put Minecraft versions, loader names, the selected base-pack name, "compatible with", "Modrinth", or sentence-style requirements in next_queries.query; those constraints are already applied by deterministic filters.
Prefer "Immersive Portals" over "Immersive Portals Fabric 1.20.1 compatibility with SpaceCraft Pluto".
Use multiple short queries instead of one long query that combines compatibility and theme constraints.
When no candidates are available for an open goal after short searches, explain the likely unresolved reason in rationale so it can be shown to the user; do not silently mark that goal covered.
Return an object matching the provided schema."#;

const BASE_COVERAGE_PROMPT: &str = r#"You are auditing whether a selected Minecraft base modpack already satisfies the user's requested features before any extra mods are searched.
You are given the user's request, the base pack's title/description, the requested theme goals (each with an id and a short label), and the base pack's own modlist (each base mod's title and provider categories).
Decide, for each theme goal, whether the base pack's existing mods already deliver that feature well enough that adding another mod would be redundant.
Mark a goal covered only when one or more named base-pack mods clearly provide it; cite those mods in covering_mods and explain briefly in rationale.
Be conservative: if the base modlist does not clearly cover a goal, leave it out so the planner can search for a dedicated mod. Do not invent base mods that are not listed.
Only return goal_id values from the provided theme goals. Return an object matching the provided schema; covered_goals may be empty."#;

/// Thin top-level agent facade.
///
/// The current implementation exposes the modpack-build capability. Future
/// capabilities should be added here as routed subworkflows/tools instead of
/// expanding one large "agent loop".
pub struct MainAgentRuntime {
    llm: AgentLlmClient,
    modpack_build: ModpackBuildWorkflow,
}

impl MainAgentRuntime {
    pub fn new(llm: AgentLlmClient) -> Self {
        Self {
            modpack_build: ModpackBuildWorkflow::new(llm.clone()),
            llm,
        }
    }

    /// Start a new agent run from a natural-language request.
    ///
    /// This is only for creating a fresh session. Existing sessions should be
    /// resumed through explicit continuation APIs so we do not re-route every
    /// approval turn as a new user intent.
    pub async fn start_new_run(&self, user_prompt: &str) -> Result<AgentRunSnapshot> {
        let intent = self.classify_intent(user_prompt).await?;
        match intent.kind {
            AgentIntentKind::BuildModpack => {
                let mut run = self.modpack_build.start(user_prompt).await?;
                run.intent = Some(intent);
                run.push_trace("main agent routed intent to modpack_build workflow");
                Ok(run)
            }
            _ => Ok(unsupported_intent_snapshot(user_prompt, intent)),
        }
    }

    pub async fn start_modpack_build(&self, user_prompt: &str) -> Result<AgentRunSnapshot> {
        let mut run = self.modpack_build.start(user_prompt).await?;
        run.intent = Some(AgentIntent {
            kind: AgentIntentKind::BuildModpack,
            confidence: 1.0,
            rationale: Some("direct modpack-build workflow entry".to_string()),
        });
        Ok(run)
    }

    pub async fn continue_modpack_build(
        &self,
        run: AgentRunSnapshot,
        decision: UserDecision,
    ) -> Result<AgentRunSnapshot> {
        self.modpack_build.continue_run(run, decision).await
    }

    pub async fn continue_from_user_message(
        &self,
        run: AgentRunSnapshot,
        user_message: &str,
    ) -> Result<AgentRunSnapshot> {
        let approval = pending_approval(&run)?;
        let route = self
            .route_approval_decision(&approval, user_message)
            .await?;
        match route {
            ApprovalRoute::Decision(decision) => {
                let mut next = self.continue_modpack_build(run, decision).await?;
                next.push_trace("main agent routed natural-language approval message");
                Ok(next)
            }
            ApprovalRoute::NeedsClarification { reason } => Ok(clarify_pending_approval_input(
                run,
                &approval,
                user_message,
                &reason,
            )),
        }
    }

    /// Drive deterministic runtime work after planning has reached an execution
    /// phase. This dispatch is intentionally phase/status based; the model does
    /// not decide whether execution should run.
    pub async fn advance(
        &self,
        run: AgentRunSnapshot,
        output_path: impl AsRef<Path>,
    ) -> Result<AgentRunSnapshot> {
        self.advance_with_executor(
            run,
            output_path,
            |approved, output_path| async move {
                execute_mrpack_build_to_path(&approved, &output_path).await
            },
            EXECUTION_RETRY_BACKOFF_BASE,
        )
        .await
    }

    async fn advance_with_executor<F, Fut>(
        &self,
        mut run: AgentRunSnapshot,
        output_path: impl AsRef<Path>,
        mut executor: F,
        retry_backoff_base: Duration,
    ) -> Result<AgentRunSnapshot>
    where
        F: FnMut(ApprovedModpackBuild, PathBuf) -> Fut,
        Fut: Future<Output = Result<serde_json::Value>>,
    {
        let output_path = output_path.as_ref().to_path_buf();
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
            let manifest = executor(approved, output_path.clone()).await?;
            let next = continue_execution_manifest_with_trace(
                run,
                manifest,
                &output_path,
                "deterministic execution tool dispatched",
                BUILD_MRPACK_ARTIFACT_TOOL,
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
                let backoff = execution_retry_backoff(retry_count, retry_backoff_base);
                if !backoff.is_zero() {
                    tokio::time::sleep(backoff).await;
                }
            } else {
                retry_count = 0;
                run = next;
            }
        }
    }

    pub fn continue_after_execution_manifest_result(
        &self,
        run: AgentRunSnapshot,
        manifest: serde_json::Value,
    ) -> Result<AgentRunSnapshot> {
        self.modpack_build
            .continue_after_execution_manifest_result(run, manifest)
    }

    async fn classify_intent(&self, user_prompt: &str) -> Result<AgentIntent> {
        let output = self
            .llm
            .prompt_typed::<AgentIntentOutput>(
                &[MAIN_AGENT_SYSTEM_PROMPT, INTENT_ROUTING_PROMPT],
                user_prompt.to_string(),
                180,
                0.0,
            )
            .await?;
        Ok(output.into_agent_intent())
    }

    async fn route_approval_decision(
        &self,
        approval: &ApprovalRequest,
        user_message: &str,
    ) -> Result<ApprovalRoute> {
        let output = self
            .llm
            .prompt_typed::<ApprovalRouteOutput>(
                &[MAIN_AGENT_SYSTEM_PROMPT, APPROVAL_DECISION_ROUTING_PROMPT],
                serde_json::json!({
                    "pending_approval": approval,
                    "latest_user_message": user_message,
                })
                .to_string(),
                260,
                0.0,
            )
            .await?;
        output.into_route(approval)
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

fn clarify_pending_approval_input(
    mut run: AgentRunSnapshot,
    approval: &ApprovalRequest,
    user_message: &str,
    reason: &str,
) -> AgentRunSnapshot {
    run.status = AgentStatus::WaitingForUser;
    run.push_message(AgentMessageKind::User, user_message.trim());
    run.push_message(
        AgentMessageKind::Assistant,
        approval_clarification_message(approval),
    );
    run.push_trace(format!(
        "approval message needed clarification at {}: {}",
        approval_kind_context_label(&approval.kind),
        reason.trim()
    ));
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

fn execution_retry_exhausted_manifest(reason: &str, attempts: u32) -> serde_json::Value {
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

pub(super) fn build_mrpack_artifact_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: BUILD_MRPACK_ARTIFACT_TOOL.to_string(),
        description: "Build the approved Modrinth .mrpack artifact at the requested output path."
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

/// Interruptible subworkflow for building a modpack from a natural-language
/// request. It owns planning and HITL gates, not final import/install/export.
pub struct ModpackBuildWorkflow {
    llm: AgentLlmClient,
}

impl ModpackBuildWorkflow {
    pub fn new(llm: AgentLlmClient) -> Self {
        Self { llm }
    }

    pub async fn start(&self, user_prompt: &str) -> Result<AgentRunSnapshot> {
        self.configure_requirements(user_prompt).await
    }

    /// Normalize hard requirements and stop before any provider/tool search.
    pub async fn configure_requirements(&self, user_prompt: &str) -> Result<AgentRunSnapshot> {
        let mut run = AgentRunSnapshot::new(user_prompt);
        run.push_trace("entered modpack_build planning subworkflow");
        run.push_message(AgentMessageKind::User, user_prompt);
        run.push_trace("created run");

        let current = BuildRestrictions::default();
        let generated = generate_restriction_update(
            &self.llm,
            user_prompt,
            &current,
            user_prompt,
            BuildRestrictionChangeSource::InitialPrompt,
        )
        .await?;
        let output = update_build_restrictions(
            Some(current),
            generated.input,
            BuildRestrictionChangeSource::InitialPrompt,
            "initial user prompt",
        )?;
        run.push_trace(format!(
            "llm generated build restriction update via {}",
            generated.model
        ));
        run.push_message(
            AgentMessageKind::Assistant,
            requirement_summary_message(&output),
        );

        let approval = requirements_approval(user_prompt, &output);
        run.status = AgentStatus::WaitingForUser;
        run.phase = AgentPhase::ConfigureRequirementsApproval;
        run.restrictions = Some(output.restrictions.clone());
        run.pending_approval = Some(approval);
        run.plan = Some(requirements_plan(user_prompt, &output));
        run.push_trace("paused at build requirements approval gate");
        Ok(run)
    }

    /// Search real modpacks and stop at the base-pack selection approval gate.
    pub async fn choose_base_pack(&self, user_prompt: &str) -> Result<AgentRunSnapshot> {
        let mut run = AgentRunSnapshot::new(user_prompt);
        run.push_trace("entered modpack_build planning subworkflow");
        run.push_message(AgentMessageKind::User, user_prompt);
        run.push_trace("created run");

        continue_to_base_pack_search(&self.llm, run).await
    }

    /// Continue from a saved waiting snapshot.
    ///
    /// Continue from one human approval gate to the next state.
    pub async fn continue_run(
        &self,
        run: AgentRunSnapshot,
        decision: UserDecision,
    ) -> Result<AgentRunSnapshot> {
        if let Some(next) = continue_modpack_build_without_model(run.clone(), decision.clone())? {
            return Ok(next);
        }
        let approval = pending_approval(&run)?;
        validate_approval_id(&approval, &decision)?;
        validate_user_decision_shape(&decision)?;

        match approval.kind {
            ApprovalKind::ConfigureRequirements => {
                if decision.kind == UserDecisionKind::Revise {
                    let feedback = decision
                        .message
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            CoreError::other("revise decision requires a feedback message")
                        })?;
                    return continue_after_requirements_feedback(&self.llm, run, feedback).await;
                }
                if decision.kind != UserDecisionKind::Approve {
                    return Err(CoreError::other(
                        "configure_requirements requires approve, revise, or cancel",
                    ));
                }
                let selected =
                    selected_approval_option(&approval, &decision, "configure_requirements")?;
                continue_after_requirements_confirmation(&self.llm, run, selected).await
            }
            ApprovalKind::ChooseBasePack | ApprovalKind::ConfirmScratchFallback => {
                if decision.kind == UserDecisionKind::Revise {
                    let feedback = decision
                        .message
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            CoreError::other("revise decision requires a feedback message")
                        })?;
                    return continue_after_base_pack_feedback(&self.llm, run, feedback).await;
                }
                if decision.kind != UserDecisionKind::Approve {
                    return Err(CoreError::other(
                        "base-pack approval requires approve, revise, or cancel",
                    ));
                }
                let selected =
                    selected_approval_option(&approval, &decision, "base-pack approval")?;
                continue_after_base_pack_choice(&self.llm, run, selected).await
            }
            ApprovalKind::ConfirmCustomization => {
                if decision.kind == UserDecisionKind::Revise {
                    let feedback = decision
                        .message
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            CoreError::other("revise decision requires a feedback message")
                        })?;
                    return continue_after_customization_feedback(
                        &self.llm, run, approval, feedback,
                    )
                    .await;
                }
                if decision.kind != UserDecisionKind::Approve {
                    return Err(CoreError::other(
                        "confirm_customization requires approve, revise, or cancel",
                    ));
                }
                let selected =
                    selected_approval_option(&approval, &decision, "confirm_customization")?;
                if selected.id == "back:choose_base_pack" {
                    return continue_after_base_pack_feedback(
                        &self.llm,
                        run,
                        "The current base pack is not suitable; return to base-pack selection",
                    )
                    .await;
                }
                continue_after_customization_confirmation(run, selected)
            }
            other => Err(CoreError::other(format!(
                "continue for approval kind {other:?} is not implemented yet"
            ))),
        }
    }

    pub fn continue_after_execution_manifest_result(
        &self,
        run: AgentRunSnapshot,
        manifest: serde_json::Value,
    ) -> Result<AgentRunSnapshot> {
        continue_after_execution_manifest_result(run, manifest)
    }
}

/// Backwards-compatible alias while the CLI/tests move from "agent runtime" to
/// "main agent + modpack-build workflow" terminology.
pub type ModpackAgentRuntime = ModpackBuildWorkflow;

/// Continue a modpack-build run when the pending decision is purely
/// deterministic. Returns `Ok(None)` when the branch needs the model.
pub fn continue_modpack_build_without_model(
    mut run: AgentRunSnapshot,
    decision: UserDecision,
) -> Result<Option<AgentRunSnapshot>> {
    let approval = pending_approval(&run)?;
    validate_approval_id(&approval, &decision)?;
    validate_user_decision_shape(&decision)?;

    if decision.kind == UserDecisionKind::Cancel {
        run.status = AgentStatus::Completed;
        run.phase = AgentPhase::Completed;
        run.pending_approval = None;
        run.push_trace("user cancelled agent run");
        return Ok(Some(run));
    }

    match approval.kind {
        ApprovalKind::ConfigureRequirements => Ok(None),
        ApprovalKind::ChooseBasePack | ApprovalKind::ConfirmScratchFallback => {
            if decision.kind != UserDecisionKind::Approve {
                return Ok(None);
            }
            let selected = selected_approval_option(&approval, &decision, "base-pack approval")?;
            if selected.id == "scratch:fallback" || selected.id == "confirm:scratch_fallback" {
                return Ok(None);
            }
            Ok(None)
        }
        ApprovalKind::ConfirmCustomization => {
            if decision.kind != UserDecisionKind::Approve {
                return Ok(None);
            }
            let selected = selected_approval_option(&approval, &decision, "confirm_customization")?;
            if selected.id == "confirm:recommended_customization" {
                return continue_after_customization_confirmation(run, selected).map(Some);
            }
            if selected.id == "back:choose_base_pack" {
                return return_to_base_pack_selection(run).map(Some);
            }
            Ok(None)
        }
        other => Err(CoreError::other(format!(
            "continue for approval kind {other:?} is not implemented yet"
        ))),
    }
}

fn return_to_base_pack_selection(mut run: AgentRunSnapshot) -> Result<AgentRunSnapshot> {
    let base_pack = current_customization_base_pack(&run)?;
    let approval = base_pack_reselection_approval(&run, base_pack);
    let plan = approval.plan.clone();
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ChooseBasePackApproval;
    run.pending_approval = Some(approval);
    run.plan = plan;
    run.approved_build = None;
    run.execution = None;
    run.mod_plan = None;
    run.tools = vec![update_build_restrictions_tool_spec()];
    run.push_message(
        AgentMessageKind::User,
        "Returned to base-pack selection from customization",
    );
    run.push_trace("user returned from customization to base-pack selection");
    Ok(run)
}

fn current_customization_base_pack(run: &AgentRunSnapshot) -> Result<serde_json::Value> {
    run.pending_approval
        .as_ref()
        .and_then(|approval| {
            approval
                .options
                .iter()
                .find(|option| option.id == "confirm:recommended_customization")
        })
        .and_then(|option| option.payload.as_ref())
        .and_then(|payload| payload.get("base_pack"))
        .cloned()
        .or_else(|| {
            run.approved_build
                .as_ref()
                .map(|build| build.base_pack.clone())
        })
        .ok_or_else(|| {
            CoreError::other("customization back action has no current base pack payload")
        })
}

fn base_pack_reselection_approval(
    run: &AgentRunSnapshot,
    base_pack: serde_json::Value,
) -> ApprovalRequest {
    let title = optional_json_string(&base_pack, "title")
        .or_else(|| optional_json_string(&base_pack, "slug"))
        .unwrap_or_else(|| "Current base pack".to_string());
    let provider =
        optional_json_string(&base_pack, "provider").unwrap_or_else(|| "modrinth".to_string());
    let project_id = optional_json_string(&base_pack, "project_id")
        .or_else(|| optional_json_string(&base_pack, "slug"))
        .unwrap_or_else(|| "current_base_pack".to_string());
    ApprovalRequest {
        id: crate::agent::state::new_id("approval"),
        kind: ApprovalKind::ChooseBasePack,
        title: "Choose a base modpack".to_string(),
        message: "Returned from customization. Keep the current base pack, or search base packs again with revised requirements.".to_string(),
        options: vec![ApprovalOption {
            id: format!("{provider}:{project_id}"),
            label: title,
            description: Some("Current base pack from the customization plan.".to_string()),
            payload: Some(base_pack),
        }],
        available_decisions: approval_decisions("Keep this base pack", "Search base packs again"),
        tools: vec![update_build_restrictions_tool_spec()],
        plan: Some(ModpackAgentPlan {
            objective: run.user_prompt.clone(),
            summary_markdown:
                "Returned to base-pack selection from customization review.".to_string(),
            risks: vec![
                "Keeping the same base pack will rerun customization planning before execution."
                    .to_string(),
            ],
            planned_actions: vec![PlannedAction {
                id: "choose-base-pack-again".to_string(),
                label: "Choose or revise the base pack before customization is planned again"
                    .to_string(),
                tool: "approval_gate".to_string(),
                args: serde_json::json!({ "kind": "choose_base_pack", "source": "customization_back" }),
                requires_approval: true,
            }],
            migration_notes: vec![],
        }),
    }
}

fn pending_approval(run: &AgentRunSnapshot) -> Result<ApprovalRequest> {
    run.pending_approval
        .clone()
        .ok_or_else(|| CoreError::other("agent session has no pending approval"))
}

fn validate_approval_id(approval: &ApprovalRequest, decision: &UserDecision) -> Result<()> {
    if approval.id != decision.approval_id {
        return Err(CoreError::other(format!(
            "approval id mismatch: expected {}, got {}",
            approval.id, decision.approval_id
        )));
    }
    Ok(())
}

fn selected_approval_option(
    approval: &ApprovalRequest,
    decision: &UserDecision,
    context: &str,
) -> Result<ApprovalOption> {
    let selected_id = decision
        .selected_option_id
        .as_deref()
        .ok_or_else(|| CoreError::other(format!("{context} requires selected_option_id")))?;
    approval
        .options
        .iter()
        .find(|o| o.id == selected_id)
        .cloned()
        .ok_or_else(|| CoreError::other(format!("unknown approval option: {selected_id}")))
}

fn validate_user_decision_shape(decision: &UserDecision) -> Result<()> {
    let has_selected_option = nonempty_opt(decision.selected_option_id.as_deref()).is_some();
    let has_message = nonempty_opt(decision.message.as_deref()).is_some();
    match decision.kind {
        UserDecisionKind::Approve => {
            if !has_selected_option {
                return Err(CoreError::other(
                    "approve decision requires selected_option_id",
                ));
            }
            if has_message {
                return Err(CoreError::other(
                    "approve decision must not include a feedback message",
                ));
            }
        }
        UserDecisionKind::Revise => {
            if has_selected_option {
                return Err(CoreError::other(
                    "revise decision must not include selected_option_id",
                ));
            }
            if !has_message {
                return Err(CoreError::other("revise decision requires message"));
            }
        }
        UserDecisionKind::Cancel => {
            if has_selected_option || has_message {
                return Err(CoreError::other(
                    "cancel decision must not include selected_option_id or message",
                ));
            }
        }
    }
    Ok(())
}

fn nonempty_opt(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn unsupported_intent_snapshot(user_prompt: &str, intent: AgentIntent) -> AgentRunSnapshot {
    let mut run = AgentRunSnapshot::new(user_prompt);
    run.workflow = AgentWorkflowKind::Unsupported;
    run.intent = Some(intent.clone());
    run.status = AgentStatus::Completed;
    run.phase = AgentPhase::Completed;
    run.push_message(AgentMessageKind::User, user_prompt);
    run.push_message(
        AgentMessageKind::Assistant,
        format!(
            "The main agent classified intent={:?}, but that capability is not wired into the workflow yet.",
            intent.kind
        ),
    );
    run.push_trace("main agent stopped at unsupported intent");
    run
}

#[derive(Debug, Clone)]
struct BasePackCandidate {
    provider: ProviderId,
    hit: SearchHit,
    matched_query: String,
    resolved_target: Option<TargetCompatibility>,
}

#[derive(Debug, Clone)]
struct SelectedBasePack {
    provider: ProviderId,
    project_id: String,
    slug: String,
    title: String,
    description: Option<String>,
}

#[derive(Debug, Clone)]
struct ModCandidate {
    provider: ProviderId,
    hit: SearchHit,
    matched_query: String,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct ResolvedModCandidate {
    candidate: ModCandidate,
    version: crate::modplatform::ProjectVersion,
    file: VersionFile,
}

#[derive(Debug, Clone)]
struct RequestedCompatibility {
    minecraft_version: Option<String>,
    loader: Option<String>,
}

#[derive(Debug, Clone)]
struct GeneratedRestrictionUpdate {
    model: String,
    input: UpdateBuildRestrictionsInput,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct GeneratedModSearchPlan {
    queries: Vec<String>,
    retain_existing_mods: bool,
    remove_existing_mod_ids: Vec<String>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct GeneratedCustomizationCritique {
    verdict: CustomizationCritiqueVerdict,
    remove_project_ids: Vec<String>,
    additional_queries: Vec<String>,
    rationale: String,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum CustomizationCritiqueVerdict {
    Pass,
    Revise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseSearchMode {
    Strict,
    Loose,
    Tight,
}

#[derive(Debug, Clone)]
struct BaseModlistCache {
    refs: Vec<ModRef>,
    source_format: String,
    fetch_count: u32,
}

#[derive(Debug, Clone)]
struct ValidatedCustomizationPlan {
    extra_mods: Vec<serde_json::Value>,
    validation: serde_json::Value,
}

#[derive(Debug, Clone)]
struct CustomizationPlanningBlocked {
    reason: String,
    replan_phase: AgentPhase,
    details: serde_json::Value,
}

#[derive(Debug, Clone)]
enum CustomizationPlanningResult {
    Validated(ValidatedCustomizationPlan),
    Blocked(CustomizationPlanningBlocked),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecutionOutcomeKind {
    Ready,
    Verifying,
    Completed,
    Blocked,
    Retry,
    Failed,
}

#[derive(Debug, Clone)]
struct ExecutionOutcome {
    kind: ExecutionOutcomeKind,
    reason: Option<String>,
    replan_phase: Option<AgentPhase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangedField {
    MinecraftVersion,
    Loader,
    VersionRequirement,
    ContentPreference,
    SearchPreference,
    BasePack,
}

fn selected_base_from_approved_build(approved: &ApprovedModpackBuild) -> Result<SelectedBasePack> {
    let label = optional_json_string(&approved.base_pack, "title")
        .unwrap_or_else(|| "Selected base pack".to_string());
    let id = match (
        optional_json_string(&approved.base_pack, "provider"),
        optional_json_string(&approved.base_pack, "project_id"),
    ) {
        (Some(provider), Some(project_id)) => format!("{provider}:{project_id}"),
        _ => "approved:base_pack".to_string(),
    };
    parse_selected_base_pack(&ApprovalOption {
        id,
        label,
        description: None,
        payload: Some(approved.base_pack.clone()),
    })
}

fn target_compatibility_from_payload(payload: &serde_json::Value) -> TargetCompatibility {
    TargetCompatibility {
        minecraft_version: optional_json_string(payload, "minecraft_version"),
        loader: optional_json_string(payload, "loader"),
        version_id: optional_json_string(payload, "base_version_id"),
        version_name: optional_json_string(payload, "base_version_name"),
        version_number: optional_json_string(payload, "base_version_number"),
        game_versions: string_array_field(payload, "base_game_versions"),
        loaders: string_array_field(payload, "base_loaders"),
        primary_file: payload
            .get("base_primary_file")
            .and_then(version_file_from_payload),
        dependencies: Vec::new(),
    }
}

fn string_array_field(value: &serde_json::Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn version_file_from_payload(value: &serde_json::Value) -> Option<VersionFile> {
    Some(VersionFile {
        url: optional_json_string(value, "url")?,
        filename: optional_json_string(value, "filename")?,
        sha1: optional_json_string(value, "sha1"),
        sha512: optional_json_string(value, "sha512"),
        size: value.get("size").and_then(|v| v.as_u64()),
        primary: value
            .get("primary")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        client_side: ProjectSideSupport::from_modrinth(
            value.get("client_side").and_then(|v| v.as_str()),
        ),
        server_side: ProjectSideSupport::from_modrinth(
            value.get("server_side").and_then(|v| v.as_str()),
        ),
    })
}

fn parse_selected_base_pack(option: &ApprovalOption) -> Result<SelectedBasePack> {
    let payload = option
        .payload
        .as_ref()
        .ok_or_else(|| CoreError::other("selected base pack option has no payload"))?;
    let provider = match payload.get("provider").and_then(|v| v.as_str()) {
        Some("modrinth") => ProviderId::Modrinth,
        Some("curseforge") => ProviderId::CurseForge,
        Some("scratch") => {
            return Ok(SelectedBasePack {
                provider: ProviderId::Modrinth,
                project_id: "scratch".to_string(),
                slug: "scratch".to_string(),
                title: json_string(payload, "title").unwrap_or_else(|_| option.label.clone()),
                description: optional_json_string(payload, "description"),
            });
        }
        other => {
            return Err(CoreError::other(format!(
                "unsupported base pack provider: {other:?}"
            )));
        }
    };
    let project_id = json_string(payload, "project_id")?;
    let slug = json_string(payload, "slug").unwrap_or_else(|_| project_id.clone());
    let title = json_string(payload, "title").unwrap_or_else(|_| option.label.clone());
    let description = optional_json_string(payload, "description");
    Ok(SelectedBasePack {
        provider,
        project_id,
        slug,
        title,
        description,
    })
}

fn json_string(value: &serde_json::Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| CoreError::other(format!("missing string payload field: {field}")))
}

fn optional_json_string(value: &serde_json::Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn planning_context_input(run: &AgentRunSnapshot) -> String {
    format!(
        "Original user request: {}\n\nTyped build restrictions:\n{}",
        run.user_prompt,
        restriction_context_text(run.restrictions.as_ref())
    )
}

fn restriction_context_text(restrictions: Option<&BuildRestrictions>) -> String {
    let Some(restrictions) = restrictions else {
        return "- minecraft_version: unspecified\n- minecraft_version_requirement: none\n- loader: unspecified\n- feature_tags: none\n- notes: none"
            .to_string();
    };
    let tags = if restrictions.feature_tags.is_empty() {
        "none".to_string()
    } else {
        restrictions.feature_tags.join(", ")
    };
    format!(
        "- revision: {}\n- minecraft_version: {}\n- minecraft_version_requirement: {}\n- loader: {}\n- feature_tags: {}\n- notes: {}",
        restrictions.revision,
        restrictions
            .minecraft_version
            .as_deref()
            .unwrap_or("unspecified"),
        restrictions
            .minecraft_version_requirement
            .as_deref()
            .unwrap_or("none"),
        restrictions.loader.as_deref().unwrap_or("unspecified"),
        tags,
        restrictions.notes.as_deref().unwrap_or("none")
    )
}

fn requested_compatibility_from_restrictions(
    restrictions: Option<&BuildRestrictions>,
) -> RequestedCompatibility {
    RequestedCompatibility {
        minecraft_version: restrictions.and_then(|r| r.minecraft_version.clone()),
        loader: restrictions.and_then(|r| r.loader.clone()),
    }
}

fn normalize_loader(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "fabric" => Some("fabric".to_string()),
        "forge" => Some("forge".to_string()),
        "neoforge" | "neo forge" => Some("neoforge".to_string()),
        "quilt" => Some("quilt".to_string()),
        _ => None,
    }
}

fn is_minecraft_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() >= 2
        && parts.len() <= 4
        && parts.first() == Some(&"1")
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests;
