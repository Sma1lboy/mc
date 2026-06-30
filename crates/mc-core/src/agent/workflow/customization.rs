use super::*;

pub(super) fn block_customization_planning(
    mut run: AgentRunSnapshot,
    blocked: CustomizationPlanningBlocked,
) -> AgentRunSnapshot {
    if blocked.replan_phase == AgentPhase::ConfigureRequirementsApproval {
        let restrictions = run.restrictions.clone().unwrap_or_default();
        let missing_fields = missing_restriction_fields(&restrictions);
        let output = UpdateBuildRestrictionsOutput {
            restrictions,
            missing_fields,
            warnings: vec![blocked.reason.clone()],
        };
        run.status = AgentStatus::WaitingForUser;
        run.phase = AgentPhase::ConfigureRequirementsApproval;
        run.pending_approval = Some(requirements_approval(&run.user_prompt, &output));
    } else {
        run.status = AgentStatus::WaitingForUser;
        run.phase = AgentPhase::ConfirmCustomizationApproval;
        run.pending_approval = Some(customization_planning_blocked_approval(&run, &blocked));
    }
    run.push_message(
        AgentMessageKind::Tool,
        format!("customization planning blocked: {}", blocked.reason),
    );
    run.push_trace("customization planning blocked; returned to HITL gate");
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
        return return_to_base_pack_selection(run);
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
    run.status = AgentStatus::Running;
    run.phase = AgentPhase::ExecutionReady;
    run.pending_approval = None;
    run.tools = vec![build_mrpack_artifact_tool_spec()];
    run.execution = Some(AgentExecutionMetadata {
        status: AgentExecutionStatus::NotStarted,
        manifest: None,
        blocked: None,
    });
    run.push_trace("approved customization plan; execution ready");
    Ok(run)
}

pub(super) async fn continue_after_customization_feedback(
    llm: &AgentLlmClient,
    mut run: AgentRunSnapshot,
    approval: ApprovalRequest,
    feedback: &str,
) -> Result<AgentRunSnapshot> {
    if let Some(replanned) =
        maybe_replan_requirements_from_feedback(llm, run.clone(), feedback).await?
    {
        return Ok(replanned);
    }

    let recommended = approval
        .options
        .iter()
        .find(|o| o.id == "confirm:recommended_customization")
        .ok_or_else(|| CoreError::other("customization approval missing recommended option"))?;
    let payload = recommended
        .payload
        .as_ref()
        .ok_or_else(|| CoreError::other("recommended customization option has no payload"))?;
    let base_pack = payload
        .get("base_pack")
        .cloned()
        .ok_or_else(|| CoreError::other("recommended customization missing base_pack"))?;
    let base = selected_base_from_customization_payload(&base_pack)?;
    let target = target_compatibility_from_payload(
        payload
            .get("target")
            .ok_or_else(|| CoreError::other("recommended customization missing target"))?,
    );
    let existing_mods = if run.mod_plan.is_some() {
        Vec::new()
    } else {
        payload
            .get("extra_mods")
            .and_then(|v| v.as_array())
            .map(|items| items.to_vec())
            .unwrap_or_default()
    };

    run.push_message(
        AgentMessageKind::User,
        format!("Changed customization requirements: {feedback}"),
    );
    run.phase = AgentPhase::CustomizationPlanning;
    run.pending_approval = None;
    run.push_trace("received customization feedback; replanning extra mods");
    merge_feedback_into_mod_plan(&mut run, feedback);

    let revised_prompt = format!(
        "{}\n\nCustomization revision feedback: {feedback}",
        planning_context_input(&run)
    );

    let base_modlist = if optional_json_string(&base_pack, "provider").as_deref() == Some("scratch")
    {
        BaseModlistCache {
            refs: Vec::new(),
            source_format: "scratch_empty".to_string(),
            fetch_count: 0,
        }
    } else {
        match fetch_base_modlist_cache(&mut run, &base_pack).await {
            Ok(cache) => cache,
            Err(err) => {
                return Ok(block_base_pack_planning(
                    run,
                    base_pack,
                    format!("Could not read the base-pack modlist: {err}"),
                ));
            }
        }
    };
    let result = run_customization_planning_loop(
        llm,
        &mut run,
        &revised_prompt,
        &base,
        &target,
        &existing_mods,
        &base_modlist,
    )
    .await?;
    let validated = match result {
        CustomizationPlanningResult::Validated(validated) => validated,
        CustomizationPlanningResult::Blocked(blocked) => {
            return Ok(block_customization_planning(run, blocked));
        }
    };
    run.push_message(
        AgentMessageKind::Tool,
        format!(
            "customization planning produced {} validated installable files using cached {} base modlist",
            validated.extra_mods.len(),
            base_modlist.source_format
        ),
    );
    let (plan, approval) = customization_approval_with_validation(
        &revised_prompt,
        &base,
        &target,
        base_pack,
        validated.extra_mods,
        Some(validated.validation),
    );
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfirmCustomizationApproval;
    run.pending_approval = Some(approval);
    run.plan = Some(plan);
    run.push_trace("paused at updated customization confirmation gate");
    Ok(run)
}

fn selected_base_from_customization_payload(
    base_pack: &serde_json::Value,
) -> Result<SelectedBasePack> {
    if optional_json_string(base_pack, "provider").as_deref() == Some("scratch") {
        return Ok(SelectedBasePack {
            provider: ProviderId::Modrinth,
            project_id: "scratch".to_string(),
            slug: "scratch".to_string(),
            title: json_str_or(base_pack, "title", "Start from scratch").to_string(),
            description: optional_json_string(base_pack, "description"),
        });
    }
    parse_selected_base_pack(&ApprovalOption {
        id: "revision:base_pack".to_string(),
        label: json_str_or(base_pack, "title", "selected base pack").to_string(),
        description: None,
        payload: Some(base_pack.clone()),
    })
}

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
    let output = llm
        .prompt_typed::<ModPlanStep>(
            &[MAIN_AGENT_SYSTEM_PROMPT, MOD_PLAN_STEP_PROMPT],
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
    Ok(output.normalized(&candidate_project_ids, &goal_ids))
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
            tool: "search_mods".into(),
            input: serde_json::json!({
                "queries": queries,
                "filters": {
                    "minecraft_version": target.minecraft_version.clone(),
                    "loader": target.loader.clone(),
                },
                "current_project_ids": current_project_ids(&state).into_iter().collect::<Vec<_>>(),
                "blocked_project_ids": state.blocked.clone(),
            }),
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
                let started = Instant::now();
                candidate_pool = search_customization_mods(&fallback_queries, target).await?;
                candidate_pool = prefilter_mod_candidates(candidate_pool, &state);
                run.push_tool_trace(AgentToolTrace {
                    event: "modplan reducer fallback_search_candidates".into(),
                    stage: AgentPhase::CustomizationPlanning,
                    iteration,
                    tool: "search_mods".into(),
                    input: serde_json::json!({
                        "queries": fallback_queries,
                        "filters": {
                            "minecraft_version": target.minecraft_version.clone(),
                            "loader": target.loader.clone(),
                        },
                    }),
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
        let step = if candidate_pool.is_empty() && !has_open_goals(&state) {
            ModPlanStep {
                selections: Vec::new(),
                removals: Vec::new(),
                next_queries: Vec::new(),
                control: ModPlanControl::Done,
                rationale: "all goals are covered".to_string(),
            }
        } else if candidate_pool.is_empty() && state.empty_candidate_rounds > 1 {
            ModPlanStep {
                selections: Vec::new(),
                removals: Vec::new(),
                next_queries: Vec::new(),
                control: ModPlanControl::Done,
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
                "control": step.control,
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

        let control = step.control;
        let applied = apply_mod_plan_step(
            &registry,
            &mut state,
            &candidate_pool,
            step,
            mc_version,
            loader,
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
        if control == ModPlanControl::Done || stopped_by_round_cap || !has_open_goals(&state) {
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

#[derive(Debug, Clone, Default)]
pub(super) struct AppliedModPlanStep {
    pub(super) selected_project_ids: Vec<String>,
    pub(super) blockers: Vec<serde_json::Value>,
}

pub(super) fn initialize_mod_plan_state(
    target: &TargetCompatibility,
    base_modlist: &BaseModlistCache,
    restrictions: Option<&BuildRestrictions>,
) -> ModPlanState {
    let loader = target.loader.as_deref().unwrap_or("unknown");
    let baseline_refs = baseline_mod_refs(loader);
    let mut goals = vec![Goal {
        id: baseline_goal_id(loader),
        label: format!("{loader} baseline"),
        kind: GoalKind::Baseline,
        status: if baseline_refs.is_empty() {
            GoalStatus::Dropped
        } else {
            GoalStatus::Open
        },
    }];
    let mut pending_queries = Vec::new();

    if let Some(restrictions) = restrictions {
        for (idx, tag) in restrictions.feature_tags.iter().enumerate() {
            let label = tag.trim();
            if label.is_empty() {
                continue;
            }
            let id = stable_goal_id("theme", label, idx);
            goals.push(Goal {
                id: id.clone(),
                label: label.to_string(),
                kind: GoalKind::Theme,
                status: GoalStatus::Open,
            });
            pending_queries.push(GoalQuery {
                goal_id: id,
                query: label.to_string(),
            });
        }
    }

    ModPlanState {
        target: target.clone(),
        base_set: base_modlist
            .refs
            .iter()
            .map(|r| ResolvedMod {
                provider: provider_slug(r.provider).to_string(),
                project_id: r.project_id.clone(),
                slug: None,
                title: None,
                version_id: None,
                filename: None,
                source_ref: None,
                payload: serde_json::json!({
                    "provider": provider_slug(r.provider),
                    "project_id": r.project_id,
                }),
                provenance: ModProvenance::BaseSet,
                goal_id: None,
            })
            .collect(),
        goals,
        additions: Vec::new(),
        removals: Vec::new(),
        blocked: Vec::new(),
        round: 0,
        empty_candidate_rounds: 0,
        pending_queries,
    }
}

fn baseline_goal_id(loader: &str) -> String {
    format!("baseline:{}", loader.trim().to_ascii_lowercase())
}

fn stable_goal_id(prefix: &str, label: &str, idx: usize) -> String {
    let slug = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        format!("{prefix}:{idx}")
    } else {
        format!("{prefix}:{slug}")
    }
}

pub(super) fn baseline_mod_refs(loader: &str) -> Vec<ModRef> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "fabric" | "quilt" => vec![ModRef::new(ProviderId::Modrinth, "P7dR8mSH")],
        _ => Vec::new(),
    }
}

fn baseline_goal_map(roots: &[ModRef], loader: &str) -> HashMap<String, String> {
    let goal_id = baseline_goal_id(loader);
    roots
        .iter()
        .map(|root| (root.key(), goal_id.clone()))
        .collect()
}

fn has_open_goals(state: &ModPlanState) -> bool {
    state
        .goals
        .iter()
        .any(|goal| goal.status == GoalStatus::Open)
}

fn open_goal_queries(state: &ModPlanState) -> Vec<GoalQuery> {
    state
        .goals
        .iter()
        .filter(|goal| goal.status == GoalStatus::Open && goal.kind == GoalKind::Theme)
        .map(|goal| GoalQuery {
            goal_id: goal.id.clone(),
            query: goal.label.clone(),
        })
        .collect()
}

fn current_project_ids(state: &ModPlanState) -> HashSet<String> {
    let removed = state.removals.iter().cloned().collect::<HashSet<_>>();
    state
        .base_set
        .iter()
        .chain(state.additions.iter())
        .filter(|m| !removed.contains(&m.project_id))
        .map(|m| m.project_id.clone())
        .collect()
}

fn installed_mod_keys(state: &ModPlanState) -> HashSet<String> {
    let removed = state.removals.iter().cloned().collect::<HashSet<_>>();
    state
        .base_set
        .iter()
        .chain(state.additions.iter())
        .filter(|m| !removed.contains(&m.project_id))
        .map(|m| provider_project_key(&m.provider, &m.project_id))
        .collect()
}

fn provider_project_key(provider: &str, project_id: &str) -> String {
    format!("{}:{}", provider.trim().to_ascii_lowercase(), project_id)
}

pub(super) fn prefilter_mod_candidates(
    candidates: Vec<ModCandidate>,
    state: &ModPlanState,
) -> Vec<ModCandidate> {
    let current = current_project_ids(state);
    let removed = state.removals.iter().cloned().collect::<HashSet<_>>();
    let blocked = state.blocked.iter().cloned().collect::<HashSet<_>>();
    candidates
        .into_iter()
        .filter(|candidate| {
            !current.contains(&candidate.hit.id)
                && !removed.contains(&candidate.hit.id)
                && !blocked.contains(&candidate.hit.id)
        })
        .collect()
}

pub(super) async fn apply_mod_plan_step(
    registry: &ProviderRegistry,
    state: &mut ModPlanState,
    candidates: &[ModCandidate],
    step: ModPlanStep,
    mc_version: &str,
    loader: &str,
) -> Result<AppliedModPlanStep> {
    for project_id in step.removals {
        if !state.removals.contains(&project_id) {
            state.removals.push(project_id.clone());
        }
        state.additions.retain(|m| m.project_id != project_id);
    }
    state.pending_queries = step.next_queries;

    let candidate_by_project = candidates
        .iter()
        .map(|candidate| (candidate.hit.id.as_str(), candidate))
        .collect::<HashMap<_, _>>();
    let root_hits = candidates
        .iter()
        .map(|candidate| (mod_ref_from_candidate(candidate).key(), candidate))
        .collect::<HashMap<_, _>>();
    let mut selected_project_ids = Vec::new();
    let mut all_blockers = Vec::new();
    for selection in step.selections {
        let Some(candidate) = candidate_by_project.get(selection.project_id.as_str()) else {
            continue;
        };
        remove_project_id(&mut state.removals, &candidate.hit.id);
        if current_project_ids(state).contains(&candidate.hit.id) {
            continue;
        }
        let root = mod_ref_from_candidate(candidate);
        selected_project_ids.push(candidate.hit.id.clone());
        let roots = vec![root.clone()];
        let mut root_goal_by_key = HashMap::new();
        root_goal_by_key.insert(root.key(), selection.goal_id.clone());

        let installed = installed_mod_keys(state);
        let resolution =
            resolve_dependencies(registry, &roots, mc_version, loader, &installed).await?;
        let blockers = customization_blockers(&resolution);
        if !blockers.is_empty() {
            push_unique_string(&mut state.blocked, candidate.hit.id.clone());
            mark_goal_status(state, &selection.goal_id, GoalStatus::Open);
            all_blockers.extend(blockers);
            continue;
        }

        remove_project_id(&mut state.blocked, &candidate.hit.id);
        append_dependency_resolution(
            state,
            &resolution,
            &roots,
            &root_hits,
            &root_goal_by_key,
            ModProvenance::Selected,
        );
    }
    Ok(AppliedModPlanStep {
        selected_project_ids,
        blockers: all_blockers,
    })
}

pub(super) fn append_dependency_resolution(
    state: &mut ModPlanState,
    resolution: &crate::modplatform::dependency::DepResolution,
    roots: &[ModRef],
    root_hits: &HashMap<String, &ModCandidate>,
    root_goal_by_key: &HashMap<String, String>,
    root_provenance: ModProvenance,
) {
    let root_keys = roots.iter().map(ModRef::key).collect::<HashSet<_>>();
    let mut current = current_project_ids(state);
    for resolved in &resolution.to_install {
        remove_project_id(&mut state.removals, &resolved.project_id);
        if current.contains(&resolved.project_id) {
            continue;
        }
        let key = ModRef::new(resolved.provider, resolved.project_id.clone()).key();
        let is_root = root_keys.contains(&key);
        let goal_id = if is_root {
            root_goal_by_key.get(&key).cloned()
        } else {
            Some(ensure_dependency_goal(state, &resolved.project_id))
        };
        if let Some(goal_id) = goal_id.as_deref() {
            mark_goal_status(state, goal_id, GoalStatus::Covered);
        }
        let payload = resolved_file_mod_payload(resolved, &root_keys, root_hits);
        if let Some(mod_entry) = resolved_mod_from_payload(
            payload,
            if is_root {
                root_provenance.clone()
            } else {
                ModProvenance::Dependency
            },
            goal_id,
        ) {
            current.insert(mod_entry.project_id.clone());
            state.additions.push(mod_entry);
        }
    }
}

fn push_unique_string(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn remove_project_id(values: &mut Vec<String>, project_id: &str) {
    values.retain(|existing| existing != project_id);
}

fn ensure_dependency_goal(state: &mut ModPlanState, project_id: &str) -> String {
    let id = format!("dependency:{project_id}");
    if !state.goals.iter().any(|goal| goal.id == id) {
        state.goals.push(Goal {
            id: id.clone(),
            label: format!("Required dependency {project_id}"),
            kind: GoalKind::Dependency,
            status: GoalStatus::Covered,
        });
    }
    id
}

fn mark_goal_status(state: &mut ModPlanState, goal_id: &str, status: GoalStatus) {
    if let Some(goal) = state.goals.iter_mut().find(|goal| goal.id == goal_id) {
        goal.status = status;
    }
}

fn resolved_mod_from_payload(
    payload: serde_json::Value,
    provenance: ModProvenance,
    goal_id: Option<String>,
) -> Option<ResolvedMod> {
    let provider = optional_json_string(&payload, "provider")?;
    let project_id = optional_json_string(&payload, "project_id")?;
    let source_ref = source_ref_payload(&payload);
    let version_id = source_ref
        .as_ref()
        .and_then(|source| optional_json_string(source, "version_id"))
        .or_else(|| {
            payload
                .get("resolved_version")
                .and_then(|v| optional_json_string(v, "version_id"))
        });
    let filename = source_ref
        .as_ref()
        .and_then(|source| source.get("file"))
        .and_then(version_file_from_payload)
        .map(|file| file.filename);
    Some(ResolvedMod {
        provider,
        project_id,
        slug: optional_json_string(&payload, "slug"),
        title: optional_json_string(&payload, "title"),
        version_id,
        filename,
        source_ref,
        payload,
        provenance,
        goal_id,
    })
}

pub(super) fn active_addition_payloads(state: &ModPlanState) -> Vec<serde_json::Value> {
    let removed = state.removals.iter().cloned().collect::<HashSet<_>>();
    state
        .additions
        .iter()
        .filter(|m| !removed.contains(&m.project_id))
        .map(|m| m.payload.clone())
        .collect()
}

async fn search_customization_mods(
    queries: &[String],
    target: &TargetCompatibility,
) -> Result<Vec<ModCandidate>> {
    const MAX_RESULTS: usize = 8;
    const MAX_PER_QUERY: usize = 2;
    const MAX_PER_QUERY_LIMIT: u32 = 2;

    let registry = ProviderRegistry::with_defaults();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let search_queries = dedupe_queries(
        queries
            .iter()
            .filter_map(|text| normalize_mod_search_query(text))
            .collect(),
    );

    for text in search_queries {
        let mut query_results = 0;
        for provider in registry.all() {
            let mut query = SearchQuery::new(text.clone(), ResourceKind::Mod);
            query.game_version = target.minecraft_version.clone();
            query.loader = target.loader.clone();
            query.limit = MAX_PER_QUERY_LIMIT;
            let provider_id = provider.id();
            for hit in provider.search(&query).await? {
                let key = format!("{provider_id:?}:{}", hit.id);
                if !seen.insert(key) {
                    continue;
                }
                out.push(ModCandidate {
                    provider: provider_id,
                    hit,
                    matched_query: text.clone(),
                });
                query_results += 1;
                if out.len() >= MAX_RESULTS {
                    return Ok(out);
                }
                if query_results >= MAX_PER_QUERY {
                    break;
                }
            }
        }
    }

    Ok(out)
}

pub(super) fn fallback_mod_search_queries(queries: &[GoalQuery]) -> Vec<String> {
    dedupe_queries(
        queries
            .iter()
            .filter_map(|query| normalize_mod_search_query(&query.query))
            .collect(),
    )
    .into_iter()
    .take(6)
    .collect()
}

pub(super) fn unresolved_mod_plan_goals(
    state: &ModPlanState,
    diagnosis: Option<String>,
) -> Vec<serde_json::Value> {
    let fallback = "No compatible provider candidates were selected for this request. Revise the request, keep the current plan, or change the target and replan.";
    state
        .goals
        .iter()
        .filter(|goal| goal.kind == GoalKind::Theme && goal.status == GoalStatus::Open)
        .map(|goal| {
            serde_json::json!({
                "goal_id": goal.id,
                "label": goal.label,
                "status": goal.status,
                "diagnosis": diagnosis
                    .as_deref()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or(fallback),
                "next_step": "Revise the request, keep the current plan, or change the target and replan.",
            })
        })
        .collect()
}

fn mod_ref_from_candidate(candidate: &ModCandidate) -> ModRef {
    ModRef::new(candidate.provider, candidate.hit.id.clone())
}

fn dependency_resolution_payload(
    resolution: &crate::modplatform::dependency::DepResolution,
) -> serde_json::Value {
    serde_json::json!({
        "to_install": resolution.to_install.iter().map(resolved_file_ref_payload).collect::<Vec<_>>(),
        "satisfied": mod_ref_payloads(&resolution.satisfied),
        "unresolved": mod_ref_payloads(&resolution.unresolved),
        "incompatible": mod_ref_payloads(&resolution.incompatible),
    })
}

pub(super) fn customization_blockers(
    resolution: &crate::modplatform::dependency::DepResolution,
) -> Vec<serde_json::Value> {
    let mut blockers = Vec::new();
    for unresolved in &resolution.unresolved {
        blockers.push(serde_json::json!({
            "kind": "unresolved_dependency",
            "provider": provider_slug(unresolved.provider),
            "project_id": unresolved.project_id.clone(),
            "reason": format!(
                "{}:{} has no compatible version or primary file",
                provider_slug(unresolved.provider),
                unresolved.project_id
            ),
        }));
    }
    for incompatible in &resolution.incompatible {
        blockers.push(serde_json::json!({
            "kind": "incompatible_dependency",
            "provider": provider_slug(incompatible.provider),
            "project_id": incompatible.project_id.clone(),
            "reason": format!(
                "{}:{} is declared incompatible by a selected mod",
                provider_slug(incompatible.provider),
                incompatible.project_id
            ),
        }));
    }
    blockers
}

fn resolved_file_ref_payload(resolved: &ResolvedFile) -> serde_json::Value {
    serde_json::json!({
        "provider": provider_slug(resolved.provider),
        "project_id": resolved.project_id.clone(),
        "version_id": resolved.version_id.clone(),
        "filename": resolved.file.filename.clone(),
    })
}

fn resolved_file_mod_payload(
    resolved: &ResolvedFile,
    root_keys: &HashSet<String>,
    root_hits: &HashMap<String, &ModCandidate>,
) -> serde_json::Value {
    let key = ModRef::new(resolved.provider, resolved.project_id.clone()).key();
    let root_hit = root_hits.get(&key).copied();
    let title = root_hit
        .map(|candidate| candidate.hit.title.clone())
        .or_else(|| resolved.project_name.clone())
        .unwrap_or_else(|| resolved.project_id.clone());
    let slug = root_hit
        .map(|candidate| candidate.hit.slug.clone())
        .or_else(|| resolved.project_slug.clone())
        .unwrap_or_else(|| resolved.project_id.clone());
    let matched_query = root_hit
        .map(|candidate| candidate.matched_query.clone())
        .unwrap_or_else(|| "dependency resolution".to_string());
    let auto_added = !root_keys.contains(&key);
    let provider = provider_slug(resolved.provider);
    let review_source = if auto_added {
        "dependency_resolution"
    } else {
        "selected_candidate"
    };
    let review_reason = if auto_added {
        "auto-added required dependency".to_string()
    } else {
        format!("matched {matched_query}")
    };
    let file = root_hit
        .map(|candidate| version_file_with_project_side(&resolved.file, &candidate.hit))
        .unwrap_or_else(|| resolved.file.clone());

    serde_json::json!({
        "provider": provider,
        "project_id": resolved.project_id.clone(),
        "slug": slug,
        "title": title,
        "description": if auto_added {
            "Automatically added required dependency"
        } else {
            "Selected extra mod"
        },
        "describe": format!(
            "{} | {} | {}",
            provider_label(resolved.provider),
            if auto_added { "auto-added dependency" } else { "selected candidate" },
            file.filename
        ),
        "author": null,
        "downloads": 0,
        "icon_url": null,
        "gallery_url": null,
        "categories": [],
        "url": project_url(resolved.provider, ResourceKind::Mod, &resolved.project_id),
        "matched_query": matched_query,
        "auto_added": auto_added,
        "dependency_reason": if auto_added { "required_dependency" } else { "root_candidate" },
        "review_source": review_source,
        "review_reason": review_reason,
        "review_version": resolved.version_id.clone(),
        "review_file": file.filename.clone(),
        "resolved_version": {
            "version_id": resolved.version_id.clone(),
            "primary_file": version_file_payload(&file),
        },
        "source_ref": {
            "kind": "mod_file",
            "provider": provider,
            "project_id": resolved.project_id.clone(),
            "version_id": resolved.version_id.clone(),
            "file": version_file_payload(&file),
        },
    })
}
