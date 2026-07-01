use super::*;

use crate::modplatform::dependency::{VersionLookupCache, resolve_dependencies_with_cache};
use futures::StreamExt;

/// 同时在途的 provider 请求上限(并发省时,又不至于猛打 provider 触发限流)。
const PROVIDER_FANOUT: usize = 8;

mod planning_state;

#[cfg(test)]
mod tests;

use planning_state::{
    active_addition_payloads, analyze_base_pack_coverage, apply_base_set_metadata,
    apply_mod_plan_step_cached, base_pack_coverage_payload, baseline_goal_map, current_project_ids,
    dependency_resolution_payload, enrich_base_set_metadata, has_open_goals, has_open_theme_goals,
    installed_mod_keys, open_goal_queries, resolved_mod_from_payload, search_customization_mods,
};

pub(super) use planning_state::{
    append_dependency_resolution, baseline_mod_refs, fallback_mod_search_queries,
    initialize_mod_plan_state, prefilter_mod_candidates, unresolved_mod_plan_goals,
};

#[cfg(test)]
use planning_state::stable_goal_id;
#[cfg(test)]
pub(super) use planning_state::{apply_mod_plan_step, customization_blockers};

pub(super) fn block_customization_planning(
    mut run: AgentRunSnapshot,
    blocked: CustomizationPlanningBlocked,
) -> AgentRunSnapshot {
    if blocked.replan_phase == AgentPhase::ConfigureRequirementsApproval {
        let output = run
            .restrictions
            .clone()
            .unwrap_or_default()
            .as_update_output(vec![blocked.reason.clone()]);
        let output_value = serde_json::to_value(&output)
            .expect("requirements planning block output should serialize");
        run.clear_user_interrupt();
        set_agent_memory(&mut run, "requirements_output", output_value);
    } else {
        let approval = customization_planning_blocked_approval(&run, &blocked);
        let plan = approval.plan.clone().or_else(|| run.plan.clone());
        let approval_value = serde_json::to_value(&approval)
            .expect("customization planning approval should serialize");
        run.clear_user_interrupt();
        run.plan = plan;
        set_agent_memory(&mut run, "confirm_customization_approval", approval_value);
    }
    run.push_message(
        AgentMessageKind::Tool,
        format!("customization planning blocked: {}", blocked.reason),
    );
    run.push_trace("customization planning blocked; stored approval draft for agent loop");
    run
}

fn customization_planning_blocked_approval(
    run: &AgentRunSnapshot,
    blocked: &CustomizationPlanningBlocked,
) -> ApprovalRequest {
    let unresolved_lines = blocked_unresolved_lines(blocked);
    let conflict_lines = blocked_conflict_lines(blocked);
    let mut message = format!(
        "Could not produce a verified compatible extra-mod plan: {}. Change the extra-mod requirements or return to base-pack selection.",
        blocked.reason
    );
    append_message_section(&mut message, "Unresolved goals", &unresolved_lines);
    append_message_section(&mut message, "Rejected candidates", &conflict_lines);
    let mut migration_notes = unresolved_lines.clone();
    migration_notes.extend(conflict_lines.clone());

    ApprovalRequest {
        id: crate::agent::state::new_id("approval"),
        kind: ApprovalKind::ConfirmCustomization,
        title: "Customization planning is blocked".to_string(),
        message,
        options: vec![ApprovalOption {
            id: "back:choose_base_pack".to_string(),
            label: "Back to base-pack selection".to_string(),
            description: Some(
                "The current base pack or requirement combination could not be verified; return to base-pack selection."
                    .to_string(),
            ),
            payload: Some(blocked_back_to_base_pack_payload(blocked)),
        }],
        available_decisions: vec![
            ApprovalDecisionSpec {
                kind: UserDecisionKind::Approve,
                label: "Back to base-pack selection".to_string(),
                requires_selected_option: true,
                requires_message: false,
            },
            ApprovalDecisionSpec {
                kind: UserDecisionKind::Cancel,
                label: "Cancel".to_string(),
                requires_selected_option: false,
                requires_message: false,
            },
        ],
        tools: Vec::new(),
        plan: Some(ModpackAgentPlan {
            objective: run.user_prompt.clone(),
            summary_markdown: format!("Customization planning is blocked: {}", blocked.reason),
            risks: vec![
                "Unverified incompatible mod plans are never advanced to execution.".to_string(),
            ],
            planned_actions: vec![PlannedAction {
                id: "revise-customization".to_string(),
                label: "User revises customization after validation block".to_string(),
                tool: "approval_gate".to_string(),
                args: serde_json::json!({ "kind": "confirm_customization", "planning_blocked": true }),
                requires_approval: true,
            }],
            migration_notes,
        }),
    }
}

fn blocked_back_to_base_pack_payload(blocked: &CustomizationPlanningBlocked) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "action": "back_to_base_pack",
        "planning_blocked": {
            "reason": blocked.reason.clone(),
            "details": blocked.details.clone(),
        }
    });
    if let Some(base_pack) = blocked.details.get("base_pack").cloned() {
        payload["base_pack"] = base_pack;
    }
    payload
}

fn append_message_section(message: &mut String, title: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    message.push('\n');
    message.push_str(title);
    message.push_str(":\n");
    message.push_str(&lines.join("\n"));
}

fn blocked_unresolved_lines(blocked: &CustomizationPlanningBlocked) -> Vec<String> {
    blocked
        .details
        .get("unresolved_goals")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let label = item.get("label").and_then(|v| v.as_str())?;
            let diagnosis = item.get("diagnosis").and_then(|v| v.as_str()).unwrap_or("");
            Some(blocked_detail_line(label, diagnosis))
        })
        .collect()
}

fn blocked_conflict_lines(blocked: &CustomizationPlanningBlocked) -> Vec<String> {
    let mut lines = blocked
        .details
        .get("last_blockers")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let project_id = item.get("project_id").and_then(|v| v.as_str())?;
            let reason = item.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            Some(blocked_detail_line(project_id, reason))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines = blocked
            .details
            .get("blocked_project_ids")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|item| item.as_str().map(|project_id| format!("- {project_id}")))
            .collect();
    }
    lines
}

fn blocked_detail_line(label: &str, detail: &str) -> String {
    if detail.trim().is_empty() {
        format!("- {label}")
    } else {
        format!("- {label}: {detail}")
    }
}

pub(super) fn continue_after_customization_confirmation(
    mut run: AgentRunSnapshot,
    selected: ApprovalOption,
) -> Result<AgentRunSnapshot> {
    if selected.id == "back:choose_base_pack" {
        return Err(CoreError::other(
            "back_to_base_pack is feedback for the agent loop, not a confirmation execution path",
        ));
    }
    if selected.id != "confirm:recommended_customization" {
        return Err(CoreError::other(format!(
            "unsupported customization option: {}",
            selected.id
        )));
    }

    let payload = selected
        .payload
        .as_ref()
        .ok_or_else(|| CoreError::other("confirmed customization option has no payload"))?;
    let build = approved_build_from_payload(payload)?;

    run.push_message(AgentMessageKind::User, "Confirmed recommended plan");
    run.push_message(
        AgentMessageKind::Assistant,
        "The plan is confirmed. The deterministic executor can compile the execution manifest next; any execution block returns to the matching approval gate.",
    );
    run.approved_build = Some(build);
    run.mod_plan = None;
    run.enter_phase(AgentPhase::ExecutionReady);
    run.tools = vec![export_mrpack_artifact_tool_spec()];
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::NotStarted,
        manifest: None,
        blocked: None,
    });
    run.push_trace("approved customization plan; execution ready");
    Ok(run)
}

#[cfg(test)]
pub(super) fn merge_feedback_into_mod_plan(run: &mut AgentRunSnapshot, feedback: &str) {
    let Some(state) = run.mod_plan.as_mut() else {
        return;
    };
    let label = feedback.trim();
    if label.is_empty() {
        return;
    }
    let id = stable_goal_id("theme", label, state.goals.len());
    if !state.goals.iter().any(|goal| goal.id == id) {
        state.goals.push(Goal {
            id: id.clone(),
            label: label.to_string(),
            kind: GoalKind::Theme,
            status: GoalStatus::Open,
        });
    }
    if !state
        .pending_queries
        .iter()
        .any(|query| query.goal_id == id)
    {
        state.pending_queries.push(GoalQuery {
            goal_id: id,
            query: label.to_string(),
        });
    }
}

#[cfg(test)]
pub(super) fn remove_existing_mod_payloads(
    existing: Vec<serde_json::Value>,
    remove_ids: &[String],
) -> Vec<serde_json::Value> {
    let remove = remove_ids
        .iter()
        .map(|id| id.trim().to_ascii_lowercase())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>();
    if remove.is_empty() {
        return existing;
    }

    existing
        .into_iter()
        .filter(|item| {
            let keys = [
                item.get("project_id").and_then(|v| v.as_str()),
                item.get("slug").and_then(|v| v.as_str()),
                item.get("title").and_then(|v| v.as_str()),
            ];
            !keys
                .iter()
                .flatten()
                .any(|key| remove.contains(&key.trim().to_ascii_lowercase()))
        })
        .collect()
}

pub(super) async fn infer_base_pack_compatibility(
    base: &SelectedBasePack,
    requested: &RequestedCompatibility,
) -> Result<TargetCompatibility> {
    let registry = ProviderRegistry::with_defaults();
    let provider = registry.get(base.provider).ok_or_else(|| {
        CoreError::other(format!("provider {:?} is not registered", base.provider))
    })?;
    let versions = provider
        .list_versions(
            &base.project_id,
            requested.minecraft_version.as_deref(),
            requested.loader.as_deref(),
        )
        .await?;
    if versions.is_empty() && (requested.minecraft_version.is_some() || requested.loader.is_some())
    {
        return Err(CoreError::other(format!(
            "selected base pack {} has no version matching requested target{}{}",
            base.title,
            requested
                .minecraft_version
                .as_ref()
                .map(|v| format!(" MC {v}"))
                .unwrap_or_default(),
            requested
                .loader
                .as_ref()
                .map(|l| format!(" / {l}"))
                .unwrap_or_default()
        )));
    }
    let versions = if versions.is_empty() {
        provider.list_versions(&base.project_id, None, None).await?
    } else {
        versions
    };
    let latest = versions.first().cloned();
    Ok(TargetCompatibility {
        minecraft_version: requested.minecraft_version.clone().or_else(|| {
            latest
                .as_ref()
                .and_then(|v| v.game_versions.first().cloned())
        }),
        loader: requested
            .loader
            .clone()
            .or_else(|| latest.as_ref().and_then(|v| v.loaders.first().cloned())),
        version_id: latest.as_ref().map(|v| v.id.clone()),
        version_name: latest.as_ref().map(|v| v.name.clone()),
        version_number: latest.as_ref().map(|v| v.version_number.clone()),
        game_versions: latest
            .as_ref()
            .map(|v| v.game_versions.clone())
            .unwrap_or_default(),
        loaders: latest
            .as_ref()
            .map(|v| v.loaders.clone())
            .unwrap_or_default(),
        primary_file: latest.as_ref().and_then(|v| v.primary_file().cloned()),
        dependencies: latest.map(|v| v.dependencies).unwrap_or_default(),
    })
}

async fn generate_mod_plan_step(
    llm: &AgentLlmClient,
    user_prompt: &str,
    base: &SelectedBasePack,
    target: &TargetCompatibility,
    state: &ModPlanState,
    candidates: &[ModCandidate],
    last_blockers: &[serde_json::Value],
) -> Result<ModPlanStep> {
    let candidate_project_ids = candidates
        .iter()
        .map(|candidate| candidate.hit.id.clone())
        .collect::<HashSet<_>>();
    let goal_ids = state
        .goals
        .iter()
        .map(|goal| goal.id.clone())
        .collect::<HashSet<_>>();
    let default_goal_id = state
        .pending_queries
        .first()
        .map(|query| query.goal_id.as_str())
        .or_else(|| {
            state
                .goals
                .iter()
                .find(|goal| goal.status == GoalStatus::Open)
                .map(|goal| goal.id.as_str())
        });
    let output = llm
        .prompt_text(
            &[
                MAIN_AGENT_SYSTEM_PROMPT,
                modpack_build_react_prompt(),
                MOD_PLAN_STEP_PROMPT,
            ],
            mod_plan_step_prompt_payload(
                user_prompt,
                base,
                target,
                state,
                candidates,
                last_blockers,
            )
            .to_string(),
            600,
            0.1,
        )
        .await?;
    parse_mod_plan_step_response(&output, &candidate_project_ids, &goal_ids, default_goal_id)
}

pub(super) fn mod_plan_step_prompt_payload(
    user_prompt: &str,
    base: &SelectedBasePack,
    target: &TargetCompatibility,
    state: &ModPlanState,
    candidates: &[ModCandidate],
    last_blockers: &[serde_json::Value],
) -> serde_json::Value {
    serde_json::json!({
        "user_prompt": user_prompt,
        "selected_base_pack": {
            "provider": provider_slug(base.provider),
            "project_id": base.project_id.clone(),
            "slug": base.slug.clone(),
            "title": base.title.clone(),
            "description": base.description.clone(),
        },
        "target": {
            "minecraft_version": target.minecraft_version.clone(),
            "loader": target.loader.clone(),
            "base_version_id": target.version_id.clone(),
            "base_version_name": target.version_name.clone(),
        },
        "round": state.round,
        "open_goals": state.goals.iter()
            .filter(|goal| goal.status == GoalStatus::Open)
            .map(|goal| serde_json::json!({
                "id": goal.id,
                "label": goal.label,
                "kind": goal.kind,
            }))
            .collect::<Vec<_>>(),
        "current_mod_set": state.additions.iter()
            .map(|m| serde_json::json!({
                "provider": m.provider,
                "project_id": m.project_id,
                "title": m.title,
                "goal_id": m.goal_id,
                "provenance": m.provenance,
            }))
            .collect::<Vec<_>>(),
        "last_blockers": last_blockers,
        "candidate_pool": candidates.iter()
            .map(|candidate| serde_json::json!({
                "provider": provider_slug(candidate.provider),
                "project_id": candidate.hit.id,
                "slug": candidate.hit.slug,
                "title": candidate.hit.title,
                "description": candidate.hit.description,
                "downloads": candidate.hit.downloads,
                "matched_query": candidate.matched_query,
                "client_side": candidate.hit.client_side,
                "server_side": candidate.hit.server_side,
            }))
            .collect::<Vec<_>>(),
    })
}

pub(super) async fn run_customization_planning_loop(
    llm: &AgentLlmClient,
    run: &mut AgentRunSnapshot,
    planning_input: &str,
    base: &SelectedBasePack,
    target: &TargetCompatibility,
    existing_mods: &[serde_json::Value],
    base_modlist: &BaseModlistCache,
) -> Result<CustomizationPlanningResult> {
    let Some(mc_version) = target
        .minecraft_version
        .as_deref()
        .filter(|s| !s.is_empty())
    else {
        return Ok(CustomizationPlanningResult::Blocked(
            CustomizationPlanningBlocked {
                reason: "missing concrete Minecraft version for dependency resolution".to_string(),
                replan_phase: AgentPhase::ConfigureRequirementsApproval,
                details: serde_json::json!({ "target": "minecraft_version" }),
            },
        ));
    };
    let Some(loader) = target.loader.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(CustomizationPlanningResult::Blocked(
            CustomizationPlanningBlocked {
                reason: "missing mod loader for dependency resolution".to_string(),
                replan_phase: AgentPhase::ConfigureRequirementsApproval,
                details: serde_json::json!({ "target": "loader" }),
            },
        ));
    };

    let registry = ProviderRegistry::with_defaults();
    let fresh_plan = run.mod_plan.is_none();
    // Run-scoped list_versions memo: the baseline seed, every per-selection dependency walk and
    // their shared transitive libs reuse it across rounds, so a given (provider, project_id, mc,
    // loader) version lookup hits the network once per planning run. Dropped when the loop returns.
    let mut version_cache = VersionLookupCache::new();
    let mut state = run.mod_plan.clone().unwrap_or_else(|| {
        initialize_mod_plan_state(target, base_modlist, run.restrictions.as_ref())
    });
    if state.additions.is_empty() && !existing_mods.is_empty() {
        state.additions = existing_mods
            .iter()
            .filter_map(|payload| {
                resolved_mod_from_payload(payload.clone(), ModProvenance::Selected, None)
            })
            .collect();
    }

    // Before the search/select loop, credit the base pack for goals it already
    // covers. Only on a fresh plan and only for real (non-scratch) base packs:
    // the scratch path starts from an empty base set, so there is nothing to
    // credit and no reason to spend an LLM round-trip.
    if fresh_plan && !state.base_set.is_empty() && has_open_theme_goals(&state) {
        let base_meta = enrich_base_set_metadata(&registry, &state.base_set).await;
        if !base_meta.is_empty() {
            apply_base_set_metadata(&mut state, &base_meta);
        }
        if let Err(err) =
            analyze_base_pack_coverage(llm, run, planning_input, base, &mut state).await
        {
            run.push_trace(format!("base-pack coverage analysis skipped: {err}"));
        }
        run.mod_plan = Some(state.clone());
    }

    let mut last_blockers = Vec::<serde_json::Value>::new();
    let mut stopped_by_round_cap = state.round >= MOD_PLAN_ROUND_CAP;
    let mut last_unresolved_diagnosis: Option<String> = None;

    while state.round < MOD_PLAN_ROUND_CAP {
        let iteration = state.round + 1;
        if state.round == 0 {
            let started = Instant::now();
            let baseline_roots = baseline_mod_refs(loader);
            if !baseline_roots.is_empty() {
                let installed = installed_mod_keys(&state);
                // Baseline runs once (round 0); its resolved mods become `already_installed` and are
                // short-circuited (never re-fetched) by every later resolve, so it gains nothing from
                // the run cache — keep the plain entry point here.
                let resolution = resolve_dependencies(
                    &registry,
                    &baseline_roots,
                    mc_version,
                    loader,
                    &installed,
                )
                .await?;
                append_dependency_resolution(
                    &mut state,
                    &resolution,
                    &baseline_roots,
                    &HashMap::new(),
                    &baseline_goal_map(&baseline_roots, loader),
                    ModProvenance::Baseline,
                );
                run.push_tool_trace(AgentToolTrace {
                    event: "modplan reducer seed_baseline".into(),
                    stage: AgentPhase::CustomizationPlanning,
                    iteration,
                    tool: "resolve_baseline_mods".into(),
                    input: serde_json::json!({
                        "roots": mod_ref_payloads(&baseline_roots),
                        "mc_version": mc_version,
                        "loader": loader,
                    }),
                    output: dependency_resolution_payload(&resolution),
                    duration_ms: started.elapsed().as_millis(),
                    status: "ok".into(),
                });
            }
        }

        let queries = if state.pending_queries.is_empty() {
            open_goal_queries(&state)
        } else {
            state.pending_queries.clone()
        };
        let search_input = serde_json::json!({
            "queries": queries.clone(),
            "filters": {
                "minecraft_version": target.minecraft_version.clone(),
                "loader": target.loader.clone(),
            },
            "current_project_ids": current_project_ids(&state).into_iter().collect::<Vec<_>>(),
            "blocked_project_ids": state.blocked.clone(),
        });
        if !queries.is_empty() {
            run.push_tool_call_started(
                AgentPhase::CustomizationPlanning,
                iteration,
                "mod_search",
                search_input.clone(),
            );
        }
        let started = Instant::now();
        let mut candidate_pool = if queries.is_empty() {
            Vec::new()
        } else {
            search_customization_mods(
                &queries
                    .iter()
                    .map(|query| query.query.clone())
                    .collect::<Vec<_>>(),
                target,
            )
            .await?
        };
        candidate_pool = prefilter_mod_candidates(candidate_pool, &state);
        run.push_tool_trace(AgentToolTrace {
            event: "modplan reducer search_candidates".into(),
            stage: AgentPhase::CustomizationPlanning,
            iteration,
            tool: "mod_search".into(),
            input: search_input,
            output: serde_json::json!({
                "count": candidate_pool.len(),
                "candidates": candidate_pool.iter().map(mod_payload).collect::<Vec<_>>(),
            }),
            duration_ms: started.elapsed().as_millis(),
            status: "ok".into(),
        });
        if candidate_pool.is_empty() && !queries.is_empty() {
            let fallback_queries = fallback_mod_search_queries(&queries);
            if !fallback_queries.is_empty() {
                let fallback_input = serde_json::json!({
                    "queries": fallback_queries.clone(),
                    "filters": {
                        "minecraft_version": target.minecraft_version.clone(),
                        "loader": target.loader.clone(),
                    },
                });
                run.push_tool_call_started(
                    AgentPhase::CustomizationPlanning,
                    iteration,
                    "mod_search",
                    fallback_input.clone(),
                );
                let started = Instant::now();
                candidate_pool = search_customization_mods(&fallback_queries, target).await?;
                candidate_pool = prefilter_mod_candidates(candidate_pool, &state);
                run.push_tool_trace(AgentToolTrace {
                    event: "modplan reducer fallback_search_candidates".into(),
                    stage: AgentPhase::CustomizationPlanning,
                    iteration,
                    tool: "mod_search".into(),
                    input: fallback_input,
                    output: serde_json::json!({
                        "count": candidate_pool.len(),
                        "candidates": candidate_pool.iter().map(mod_payload).collect::<Vec<_>>(),
                    }),
                    duration_ms: started.elapsed().as_millis(),
                    status: "ok".into(),
                });
            }
        }
        if candidate_pool.is_empty() {
            state.empty_candidate_rounds = state.empty_candidate_rounds.saturating_add(1);
        } else {
            state.empty_candidate_rounds = 0;
        }

        let started = Instant::now();
        let step = if candidate_pool.is_empty() && !has_open_theme_goals(&state) {
            ModPlanStep {
                selections: Vec::new(),
                removals: Vec::new(),
                next_queries: Vec::new(),
                rationale: "all theme goals are covered".to_string(),
            }
        } else if candidate_pool.is_empty() && state.empty_candidate_rounds > 1 {
            ModPlanStep {
                selections: Vec::new(),
                removals: Vec::new(),
                next_queries: Vec::new(),
                rationale:
                    "no compatible provider candidates were found for the current open goals"
                        .to_string(),
            }
        } else {
            generate_mod_plan_step(
                llm,
                planning_input,
                base,
                target,
                &state,
                &candidate_pool,
                &last_blockers,
            )
            .await?
        };
        run.push_tool_trace(AgentToolTrace {
            event: "modplan reducer model_step".into(),
            stage: AgentPhase::CustomizationPlanning,
            iteration,
            tool: "mod_plan_step".into(),
            input: serde_json::json!({
                "candidate_count": candidate_pool.len(),
                "open_goals": state.goals.iter()
                    .filter(|goal| goal.status == GoalStatus::Open)
                    .map(|goal| goal.id.clone())
                    .collect::<Vec<_>>(),
            }),
            output: serde_json::json!({
                "model": llm.model(),
                "selections": step.selections.iter()
                    .map(|selection| serde_json::json!({
                        "goal_id": selection.goal_id,
                        "project_id": selection.project_id,
                    }))
                    .collect::<Vec<_>>(),
                "removals": step.removals.clone(),
                "next_queries": step.next_queries.clone(),
                "rationale": step.rationale.clone(),
            }),
            duration_ms: started.elapsed().as_millis(),
            status: "ok".into(),
        });
        if candidate_pool.is_empty() && has_open_goals(&state) {
            let diagnosis = step.rationale.trim();
            if !diagnosis.is_empty() {
                last_unresolved_diagnosis = Some(diagnosis.to_string());
            }
        }

        let applied = apply_mod_plan_step_cached(
            &registry,
            &mut state,
            &candidate_pool,
            step,
            mc_version,
            loader,
            &mut version_cache,
        )
        .await?;
        if !applied.blockers.is_empty() {
            last_blockers = applied.blockers.clone();
            run.push_tool_trace(AgentToolTrace {
                event: "modplan reducer detect_conflicts".into(),
                stage: AgentPhase::CustomizationPlanning,
                iteration,
                tool: "detect_conflicts".into(),
                input: serde_json::json!({ "selected_project_ids": applied.selected_project_ids }),
                output: serde_json::json!({ "blockers": applied.blockers }),
                duration_ms: 0,
                status: "blocked".into(),
            });
        }

        state.round += 1;
        run.mod_plan = Some(state.clone());
        if state.round >= MOD_PLAN_ROUND_CAP {
            stopped_by_round_cap = true;
        }
        if stopped_by_round_cap
            || !has_open_theme_goals(&state)
            || (candidate_pool.is_empty() && state.empty_candidate_rounds > 1)
        {
            break;
        }
    }

    let extra_mods = active_addition_payloads(&state);
    let unresolved_goals = unresolved_mod_plan_goals(&state, last_unresolved_diagnosis);
    if stopped_by_round_cap && !unresolved_goals.is_empty() {
        run.mod_plan = Some(state.clone());
        return Ok(round_cap_blocked_result(
            base,
            &state,
            unresolved_goals,
            last_blockers,
        ));
    }
    let validation = serde_json::json!({
        "status": "validated",
        "base_modlist": {
            "source_format": base_modlist.source_format.clone(),
            "mod_refs": mod_ref_payloads(&base_modlist.refs),
            "fetch_count": base_modlist.fetch_count,
        },
        "mod_plan": {
            "round": state.round,
            "round_cap": MOD_PLAN_ROUND_CAP,
            "round_limit": MOD_PLAN_ROUND_CAP,
            "goals": state.goals.clone(),
            "removals": state.removals.clone(),
            "blocked_project_ids": state.blocked.clone(),
            "last_blockers": last_blockers,
        },
        "auto_added_dependencies": extra_mods
            .iter()
            .filter(|m| m.get("auto_added").and_then(|v| v.as_bool()).unwrap_or(false))
            .cloned()
            .collect::<Vec<_>>(),
        "base_pack_coverage": base_pack_coverage_payload(&state),
        "unresolved_goals": unresolved_goals,
    });
    Ok(CustomizationPlanningResult::Validated(
        ValidatedCustomizationPlan {
            extra_mods,
            validation,
        },
    ))
}

fn round_cap_blocked_result(
    base: &SelectedBasePack,
    state: &ModPlanState,
    unresolved_goals: Vec<serde_json::Value>,
    last_blockers: Vec<serde_json::Value>,
) -> CustomizationPlanningResult {
    CustomizationPlanningResult::Blocked(CustomizationPlanningBlocked {
        reason: format!(
            "mod planning reached round cap {MOD_PLAN_ROUND_CAP} with unresolved goals"
        ),
        replan_phase: AgentPhase::ConfirmCustomizationApproval,
        details: serde_json::json!({
            "round": state.round,
            "round_cap": MOD_PLAN_ROUND_CAP,
            "unresolved_goals": unresolved_goals,
            "blocked_project_ids": state.blocked.clone(),
            "last_blockers": last_blockers,
            "base_pack": selected_base_pack_payload(base),
        }),
    })
}

fn selected_base_pack_payload(base: &SelectedBasePack) -> serde_json::Value {
    serde_json::json!({
        "provider": provider_slug(base.provider),
        "project_id": base.project_id.clone(),
        "slug": base.slug.clone(),
        "title": base.title.clone(),
        "description": base.description.clone(),
    })
}
