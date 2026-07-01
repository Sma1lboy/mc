//! Main-agent workflow entry points and the modpack-build tool runner.
//!
//! The top-level agent routes user intents into prompt-guided tool loops. The
//! model chooses the next tool from the modpack-build catalog; Rust owns the
//! deterministic tools, approval interrupts, and artifact execution boundary.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::{CoreError, Result};
use crate::modpack::export::modrinth::host_in_whitelist;
use crate::modplatform::dependency::{ModRef, resolve_dependencies};
use crate::modplatform::provider::ProviderRegistry;
use crate::modplatform::{
    ProjectSideSupport, ProviderId, ResolvedFile, ResourceKind, SearchHit, SearchQuery, SortMethod,
    VersionFile,
};
use serde::Deserialize;

use super::llm::AgentLlmClient;
use super::state::{
    AgentEntry, AgentExecutionMetadata, AgentExecutionStatus, AgentInputKind, AgentInputOption,
    AgentIntent, AgentIntentKind, AgentInterrupt, AgentInterruptKind, AgentLaunchContext,
    AgentMessageKind, AgentPhase, AgentRunSnapshot, AgentStatus, AgentStreamEventKind,
    AgentToolSpec, AgentToolTrace, AgentWorkflowId, AgentWorkflowKind, ApprovalDecisionSpec,
    ApprovalKind, ApprovalOption, ApprovalRequest, ApprovedModpackBuild,
    BuildRestrictionChangeSource, BuildRestrictionPatch, BuildRestrictions, ExecutionBlocked, Goal,
    GoalKind, GoalQuery, GoalStatus, ModPlanState, ModProvenance, ModpackAgentPlan, PlanArtifact,
    PlanReplanRequest, PlannedAction, ResolvedMod, TargetCompatibility,
    UpdateBuildRestrictionsInput, UpdateBuildRestrictionsOutput, UserDecision, UserDecisionKind,
    is_minecraft_version, normalize_loader,
};
// `BuildRestrictionChange` now only appears in restriction tests; the patch
// application that used to build it lives in `state.rs` on `try_apply`.
#[cfg(test)]
use super::state::BuildRestrictionChange;

mod agent_loop;
mod approvals;
mod artifacts;
mod base_modlist;
mod base_search;
mod customization;
mod execution;
mod execution_tool;
mod llm_io;
mod react;
mod requirements;
mod types;

use artifacts::{
    attach_base_pack_resolution, candidate_option, customization_approval,
    customization_approval_with_validation, mod_payload, mrpack_file_payload_with_filename,
    project_url, provider_label, provider_slug, safe_provider_filename, scratch_base_pack_option,
    scratch_base_pack_payload, scratch_build_plan, selection_plan, source_ref_payload,
    version_file_payload, version_file_with_project_side,
};

use approvals::{
    approval_decisions, approved_build_from_payload, base_pack_selection_approval,
    requirement_summary_message, requirements_approval, requirements_plan,
    restrictions_from_requirement_payload,
};
#[cfg(test)]
use artifacts::{mrpack_file_payload, resolved_mod_payload};
use base_modlist::{fetch_base_modlist_cache, mod_ref_payloads};
#[cfg(test)]
use base_search::{base_search_has_acceptable_count, next_base_search_mode};
use base_search::{plan_customization_after_base_pack_choice, run_base_pack_search_loop};
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
    infer_base_pack_compatibility, run_customization_planning_loop,
};
#[cfg(test)]
use llm_io::ModSelection;
#[cfg(test)]
use llm_io::parse_approval_decision_response;
#[cfg(test)]
use llm_io::parse_mod_query_response;
#[cfg(test)]
use llm_io::search_queries;
use llm_io::{
    ApprovalRoute, ModPlanStep, dedupe_queries, normalize_mod_search_query,
    parse_approval_route_response, parse_base_coverage_response, parse_intent_response,
    parse_mod_plan_step_response, update_build_restrictions_tool_spec,
};
#[cfg(test)]
use react::modpack_build_react_tool_specs;
use react::{begin_modpack_build_react_run, modpack_build_react_prompt};
#[cfg(test)]
use requirements::{
    ALL_CHANGED_FIELDS, apply_requirements_replan, invalidation_rule_for_changed_field,
    normalize_restriction_update_input, parse_restriction_update_response,
    restriction_update_request_payload, target_phase_for_changed_field,
    validate_restriction_update_retry,
};
use requirements::{invalidate_downstream, update_build_restrictions};

#[cfg(test)]
use base_modlist::{
    base_modlist_cache_from_archive_bytes, ensure_base_archive_size, parse_base_modlist,
};

#[cfg(test)]
use agent_loop::apply_extracted_modpack_goals;
use agent_loop::set_agent_memory;
#[cfg(test)]
use agent_loop::{ModpackAgentInputKind, request_modpack_agent_input};
use agent_loop::{
    ModpackBuildAgent, pending_approval, pending_user_input, validate_user_decision_shape,
};
pub use agent_loop::{apply_modpack_build_user_decision, apply_modpack_build_user_input};
use execution::verify_written_mrpack;
pub use execution::{
    MrpackExecutionBuild, MrpackOverrideFile, build_mrpack_from_base_archive_bytes,
    compile_mrpack_execution_metadata, continue_after_execution_manifest_result,
    execute_mrpack_build_to_path,
};
#[cfg(test)]
use execution_tool::execution_retry_exhausted_manifest;
use execution_tool::run_export_mrpack_artifact_tool;
use execution_tool::{clarify_pending_approval_input, export_mrpack_artifact_tool_spec};
use types::*;

const UPDATE_BUILD_RESTRICTIONS_TOOL: &str = "update_build_restrictions";
const EXTRACT_MODPACK_GOALS_TOOL: &str = "extract_modpack_goals";
pub const EXPORT_MRPACK_ARTIFACT_TOOL: &str = "export_mrpack_artifact";
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
const MODPACK_AGENT_MAX_TURNS: u32 = 12;

const MAIN_AGENT_SYSTEM_PROMPT: &str = r#"You are the local AI agent for a Minecraft launcher.
Your job is to turn user requests into safe daemon-owned workflows, not to perform game file writes directly.

Current capabilities and boundaries:
- Route each user request before taking workflow action.
- For modpack creation, prefer recommending an existing base modpack, then customize it with compatible existing mods.
- If no base pack is suitable, the workflow may offer a human-approved scratch fallback that starts from an empty base set and uses deterministic dependency resolution.
- Search, import, install, export, and file writes are deterministic daemon tools; do not invent platform results, files, versions, or installation state.
- Stop at human approval gates before choosing a base pack, approving customization, or starting any write/install/export execution.
- General mod/modpack how-to questions are not handled by this local workflow yet; route unsupported requests to unknown.
- When a subtask asks for JSON, return only the requested JSON object without markdown or commentary."#;

const INTENT_ROUTING_PROMPT_HEADER: &str = r#"Classify the user's request into exactly one intent.
Only choose from workflows listed as available for the current agent entry.
Return unknown for unsupported, unavailable, ambiguous, or unrelated requests."#;

const APPROVAL_DECISION_ROUTING_PROMPT: &str = r#"Convert the user's latest message into a decision for the current pending approval gate.
Use only the current approval's kind, available decisions, options, and tool schemas.
Return approve only when the user clearly accepts one available option. If the user refers to an option by ordinal words like first/second/third, map it to the matching option id.
Return revise when the user asks to change, replace, search again, add requirements, remove requirements, or otherwise modify the current proposal.
Return cancel when the user clearly asks to stop or cancel.
Return needs_clarification when the message is ambiguous, asks an unrelated question, or cannot be mapped to the current approval gate.
Do not skip future workflow gates. If the user mentions future-stage requirements, preserve them in revise feedback instead of jumping ahead.
Return exactly one JSON object with decision, selected_option_id, message, and rationale."#;

#[cfg(test)]
const REQUIREMENT_NORMALIZATION_PROMPT: &str = r#"Generate arguments for the update_build_restrictions tool.
Do not search for modpacks or mods.
Do not choose default values for missing fields.
Only set minecraft_version when the user explicitly gives a concrete Minecraft version such as 1.20.1.
Set minecraft_version_requirement to the raw user-facing version requirement when present, including concrete versions and ranges such as 1.20.x, <=1.19.x, or 1.20.1/1.20.4.
Only set loader to fabric, forge, neoforge, or quilt when the user explicitly asks for that loader.
Use null when the loader is absent or ambiguous.
Feature tags must be short, lowercase English search keywords (for example: exploration, dungeons, magic, minimap, inventory-management), even when the user writes in Chinese or another language — these tags are used verbatim as English mod-search queries against Modrinth. Translate the user's intent into English keywords; never emit tags in the user's language, and never write full sentences.
The patch represents the full desired BuildRestrictions state after applying the latest user message, not only a delta.
Return exactly one JSON object using the update_build_restrictions tool argument shape."#;

#[cfg(test)]
const REQUIREMENT_NORMALIZATION_RETRY_PROMPT: &str = r#"The previous response was not valid update_build_restrictions tool arguments.
Return exactly one JSON object for update_build_restrictions.
Do not return multiple objects, markdown fences, explanations, or copied previous output."#;

#[cfg(test)]
const SEARCH_QUERY_PROMPT: &str = r#"You are planning the base-pack search step for a Minecraft modpack build workflow.
Return short English search queries for finding an existing base modpack.
Prefer canonical project/mod names or well-known ecosystem terms implied by the user's request over broad category phrases.
Include specific requirement keywords that are likely to appear in project titles or descriptions.
Each query must be a concise platform search string, not a sentence.
Use separate short queries instead of one long query that combines every requirement.
Across the query set, cover every major user-requested feature instead of focusing on only one theme.
Do not include generic words like "Minecraft", "modpack", "base pack", or "pack"; the search tool already filters for modpacks.
Prefer mature base modpacks. The scratch fallback is a later human-approved option, not a search query target.
Return exactly one JSON object: {"queries":["short query"]}."#;

const MOD_PLAN_STEP_PROMPT: &str = r#"You are planning a compatible Minecraft mod set.
The runtime has already searched provider candidates and filtered obvious duplicates.
Select only project_id values from the provided candidate pool. Do not invent URLs, version ids, filenames, hashes, or environment metadata.
Use selections to cover open goals, removals to drop unwanted current additions, and next_queries for the next deterministic search round.
When candidate_pool is empty or insufficient, next_queries must be short English provider search terms: canonical mod/project names or 2-5 English keyword phrases, even when the user writes in another language.
Do not put Minecraft versions, loader names, the selected base-pack name, "compatible with", "Modrinth", or sentence-style requirements in next_queries.query; those constraints are already applied by deterministic filters.
Prefer "Immersive Portals" over "Immersive Portals Fabric 1.20.1 compatibility with SpaceCraft Pluto".
Use multiple short queries instead of one long query that combines compatibility and theme constraints.
When no candidates are available for an open goal after short searches, explain the likely unresolved reason in rationale so it can be shown to the user; do not silently mark that goal covered.
Return exactly one JSON object with selections, removals, next_queries, and rationale."#;

const BASE_COVERAGE_PROMPT: &str = r#"You are auditing whether a selected Minecraft base modpack already satisfies the user's requested features before any extra mods are searched.
You are given the user's request, the base pack's title/description, the requested theme goals (each with an id and a short label), and the base pack's own modlist (each base mod's title and provider categories).
Decide, for each theme goal, whether the base pack's existing mods already deliver that feature well enough that adding another mod would be redundant.
Mark a goal covered only when one or more named base-pack mods clearly provide it; cite those mods in covering_mods and explain briefly in rationale.
Be conservative: if the base modlist does not clearly cover a goal, leave it out so the planner can search for a dedicated mod. Do not invent base mods that are not listed.
Only return goal_id values from the provided theme goals. Return exactly one JSON object with covered_goals; covered_goals may be empty."#;

const MODPACK_BUILD_TOOL_LOOP_PROMPT: &str = r#"You are the modpack_build agent loop.

Follow the workflow in the ReAct prompt by choosing one next action at a time. Return exactly one JSON object and no markdown.

Allowed action shapes:
- {"action":"tool_call","tool":"update_build_restrictions","args":{...},"rationale":"..."}
- {"action":"tool_call","tool":"extract_modpack_goals","args":{"goals":["short goal"]},"rationale":"..."}
- {"action":"tool_call","tool":"modpack_search","args":{"queries":["short query"]},"rationale":"..."}
- {"action":"tool_call","tool":"plan_customization","args":{},"rationale":"..."}
- {"action":"request_input","input_kind":"select_minecraft_version","args":{"version_request":"1.20.x","candidates":["1.20.4","1.20.1"],"loader":"fabric"},"rationale":"..."}
- {"action":"request_approval","approval_kind":"configure_requirements","rationale":"..."}
- {"action":"request_approval","approval_kind":"choose_base_pack","rationale":"..."}
- {"action":"request_approval","approval_kind":"confirm_customization","rationale":"..."}
- {"action":"final","message":"..."}

Rules:
- The workflow is in this prompt, not in Rust phases. Do not emit next_state, should_advance, or phase.
- Use tool_call for provider facts, compatibility, planning, and artifact execution.
- Request missing user input through request_input, not through a tool call.
- Request approval through request_approval, not through a tool call.
- If the user gave an ambiguous Minecraft version range or no concrete Minecraft version, use request_input/select_minecraft_version so the UI can render a version picker.
- After update_build_restrictions prepares requirements, call extract_modpack_goals for explicit required features, then request configure_requirements approval.
- After modpack_search prepares base-pack candidates, request choose_base_pack approval.
- After plan_customization prepares a customization approval draft, request confirm_customization approval.
- Do not build/export files until the user has confirmed the customization plan and the caller supplies an output path."#;

fn intent_routing_prompt(launch_context: &AgentLaunchContext) -> String {
    let entry = match &launch_context.entry {
        AgentEntry::Home => "home".to_string(),
    };
    let mut lines = vec![
        INTENT_ROUTING_PROMPT_HEADER.to_string(),
        String::new(),
        format!("Current agent entry: {entry}"),
        "Available workflows:".to_string(),
    ];
    for workflow in &launch_context.available_workflows {
        lines.push(format!("- {}", workflow_prompt_line(workflow)));
    }
    lines.push(
        "- unknown: anything else, including requests that require a workflow not listed above."
            .to_string(),
    );
    lines.push(String::new());
    lines
        .push("Return exactly one JSON object with intent, confidence, and rationale.".to_string());
    lines.join("\n")
}

fn workflow_prompt_line(workflow: &AgentWorkflowId) -> &'static str {
    match workflow {
        AgentWorkflowId::BuildModpack => {
            "build_modpack: create, customize, recommend, or generate a new modpack from requirements."
        }
    }
}

fn intent_available_in_context(intent: &AgentIntent, launch_context: &AgentLaunchContext) -> bool {
    intent
        .kind
        .workflow_id()
        .is_some_and(|workflow| launch_context.allows_workflow(workflow))
}

/// Thin top-level agent facade.
///
/// The current implementation exposes the modpack-build capability. Future
/// capabilities should be added here as routed agents/tools instead of
/// expanding one large "agent loop".
pub struct MainAgentRuntime {
    llm: AgentLlmClient,
    modpack_build: ModpackBuildAgent,
}

impl MainAgentRuntime {
    pub fn new(llm: AgentLlmClient) -> Self {
        Self {
            modpack_build: ModpackBuildAgent::new(llm.clone()),
            llm,
        }
    }

    /// Start a new agent run from a natural-language request.
    ///
    /// This is only for creating a fresh session. Existing sessions should be
    /// resumed through explicit continuation APIs so we do not re-route every
    /// approval turn as a new user intent.
    pub async fn start_new_run(&self, user_prompt: &str) -> Result<AgentRunSnapshot> {
        self.start_new_run_with_entry(user_prompt, AgentEntry::Home)
            .await
    }

    pub async fn start_new_run_with_entry(
        &self,
        user_prompt: &str,
        entry: AgentEntry,
    ) -> Result<AgentRunSnapshot> {
        self.start_new_run_with_context(user_prompt, AgentLaunchContext::from_entry(entry))
            .await
    }

    async fn start_new_run_with_context(
        &self,
        user_prompt: &str,
        launch_context: AgentLaunchContext,
    ) -> Result<AgentRunSnapshot> {
        let intent = self.classify_intent(user_prompt, &launch_context).await?;
        if !intent_available_in_context(&intent, &launch_context) {
            return Ok(unsupported_intent_snapshot(
                user_prompt,
                intent,
                launch_context,
            ));
        }
        match intent.kind {
            AgentIntentKind::BuildModpack => {
                let mut run = self.modpack_build.start(user_prompt).await?;
                run.launch_context = launch_context;
                run.intent = Some(intent);
                run.push_trace("main agent routed intent to modpack_build agent");
                Ok(run)
            }
            _ => Ok(unsupported_intent_snapshot(
                user_prompt,
                intent,
                launch_context,
            )),
        }
    }

    pub async fn start_modpack_build(&self, user_prompt: &str) -> Result<AgentRunSnapshot> {
        let mut run = self.modpack_build.start(user_prompt).await?;
        run.launch_context = AgentLaunchContext::from_entry(AgentEntry::Home);
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
        if let Some(interrupt) = run
            .pending_interrupt
            .clone()
            .filter(|interrupt| interrupt.kind == AgentInterruptKind::UserInput)
        {
            let mut next = self
                .continue_from_user_input(run, &interrupt.resume_token, user_message)
                .await?;
            next.push_trace("main agent routed natural-language input message");
            return Ok(next);
        }

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

    pub async fn continue_from_user_input(
        &self,
        run: AgentRunSnapshot,
        resume_token: &str,
        value: &str,
    ) -> Result<AgentRunSnapshot> {
        let interrupt = pending_user_input(&run)?;
        if interrupt.resume_token != resume_token {
            return Err(CoreError::other(format!(
                "interrupt resume token mismatch: expected {}, got {}",
                interrupt.resume_token, resume_token
            )));
        }
        let mut next = apply_modpack_build_user_input(run, &interrupt, value)?;
        next = self.modpack_build.run_tool_loop(next).await?;
        next.push_trace("main agent resumed structured user input");
        Ok(next)
    }

    pub async fn execute_tool(
        &self,
        run: AgentRunSnapshot,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<AgentRunSnapshot> {
        match tool {
            EXPORT_MRPACK_ARTIFACT_TOOL => run_export_mrpack_artifact_tool(run, args).await,
            other => Err(CoreError::other(format!("unsupported agent tool: {other}"))),
        }
    }

    pub fn continue_after_execution_manifest_result(
        &self,
        run: AgentRunSnapshot,
        manifest: serde_json::Value,
    ) -> Result<AgentRunSnapshot> {
        continue_after_execution_manifest_result(run, manifest)
    }

    async fn classify_intent(
        &self,
        user_prompt: &str,
        launch_context: &AgentLaunchContext,
    ) -> Result<AgentIntent> {
        let routing_prompt = intent_routing_prompt(launch_context);
        let output = self
            .llm
            .prompt_text(
                &[MAIN_AGENT_SYSTEM_PROMPT, routing_prompt.as_str()],
                user_prompt.to_string(),
                180,
                0.0,
            )
            .await?;
        parse_intent_response(&output).ok_or_else(|| {
            CoreError::other(format!(
                "could not parse intent JSON from model output: {output}"
            ))
        })
    }

    async fn route_approval_decision(
        &self,
        approval: &ApprovalRequest,
        user_message: &str,
    ) -> Result<ApprovalRoute> {
        let output = self
            .llm
            .prompt_text(
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
        parse_approval_route_response(&output, approval)
    }
}

/// materializes runtime approval interrupts.
fn nonempty_opt(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn unsupported_intent_snapshot(
    user_prompt: &str,
    intent: AgentIntent,
    launch_context: AgentLaunchContext,
) -> AgentRunSnapshot {
    let mut run = AgentRunSnapshot::new(user_prompt);
    run.workflow = AgentWorkflowKind::Unsupported;
    run.launch_context = launch_context;
    run.intent = Some(intent.clone());
    run.complete(AgentPhase::Completed);
    run.push_message(AgentMessageKind::User, user_prompt);
    run.push_message(
        AgentMessageKind::Assistant,
        format!(
            "The main agent classified intent={:?}, but that capability is not available from the current agent entry or is not wired into a workflow yet.",
            intent.kind
        ),
    );
    run.push_trace("main agent stopped at unsupported intent");
    run
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

#[cfg(test)]
mod tests;
