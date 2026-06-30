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
    ApprovalRequest {
        id: crate::agent::state::new_id("approval"),
        kind: ApprovalKind::ConfirmCustomization,
        title: "定制规划受阻，需要修改".to_string(),
        message: format!(
            "无法产出已验证兼容的补充 mod 方案: {}。请修改补充需求，或返回重选底包。",
            blocked.reason
        ),
        options: vec![ApprovalOption {
            id: "back:choose_base_pack".to_string(),
            label: "返回重选底包".to_string(),
            description: Some("当前底包或需求组合无法验证，回到底包选择。".to_string()),
            payload: Some(serde_json::json!({
                "action": "back_to_base_pack",
                "planning_blocked": {
                    "reason": blocked.reason.clone(),
                    "details": blocked.details.clone(),
                }
            })),
        }],
        available_decisions: vec![
            ApprovalDecisionSpec {
                kind: UserDecisionKind::Approve,
                label: "返回重选底包".to_string(),
                requires_selected_option: true,
                requires_message: false,
            },
            ApprovalDecisionSpec {
                kind: UserDecisionKind::Cancel,
                label: "取消".to_string(),
                requires_selected_option: false,
                requires_message: false,
            },
        ],
        tools: Vec::new(),
        plan: Some(ModpackAgentPlan {
            objective: run.user_prompt.clone(),
            summary_markdown: format!("定制规划受阻: {}", blocked.reason),
            risks: vec!["不会把未验证兼容的 mod 方案推进到执行阶段。".to_string()],
            planned_actions: vec![PlannedAction {
                id: "revise-customization".to_string(),
                label: "User revises customization after validation block".to_string(),
                tool: "approval_gate".to_string(),
                args: serde_json::json!({ "kind": "confirm_customization", "planning_blocked": true }),
                requires_approval: true,
            }],
            migration_notes: vec![],
        }),
    }
}

pub(super) fn continue_after_customization_confirmation(
    mut run: AgentRunSnapshot,
    selected: ApprovalOption,
) -> Result<AgentRunSnapshot> {
    if selected.id == "back:choose_base_pack" {
        return Err(CoreError::other(
            "returning to base-pack selection is not implemented in the MVP session state",
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

    run.push_message(AgentMessageKind::User, "确认推荐方案");
    run.push_message(
        AgentMessageKind::Assistant,
        "方案已确认。下一步由确定性执行器编译执行清单；如执行清单受阻，将显式回到对应审核 gate。",
    );
    run.approved_build = Some(build);
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
    openai: &OpenAiClient,
    mut run: AgentRunSnapshot,
    approval: ApprovalRequest,
    feedback: &str,
) -> Result<AgentRunSnapshot> {
    if let Some(replanned) =
        maybe_replan_requirements_from_feedback(openai, run.clone(), feedback).await?
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
    let base = parse_selected_base_pack(&ApprovalOption {
        id: "revision:base_pack".to_string(),
        label: json_str_or(&base_pack, "title", "selected base pack").to_string(),
        description: None,
        payload: Some(base_pack.clone()),
    })?;
    let target = target_compatibility_from_payload(
        payload
            .get("target")
            .ok_or_else(|| CoreError::other("recommended customization missing target"))?,
    );
    let existing_mods = payload
        .get("extra_mods")
        .and_then(|v| v.as_array())
        .map(|items| items.to_vec())
        .unwrap_or_default();

    run.push_message(AgentMessageKind::User, format!("修改定制需求: {feedback}"));
    run.phase = AgentPhase::CustomizationPlanning;
    run.pending_approval = None;
    run.push_trace("received customization feedback; replanning extra mods");

    let revised_prompt = format!(
        "{}\n\nCustomization revision feedback: {feedback}",
        planning_context_input(&run)
    );

    let base_modlist = match fetch_base_modlist_cache(&mut run, &base_pack).await {
        Ok(cache) => cache,
        Err(err) => {
            return Ok(block_base_pack_planning(
                run,
                base_pack,
                format!("无法读取底包 modlist: {err}"),
            ));
        }
    };
    let result = run_customization_planning_loop(
        openai,
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

fn existing_mod_summaries(mods: &[serde_json::Value]) -> Vec<serde_json::Value> {
    mods.iter()
        .map(|m| {
            serde_json::json!({
                "provider": optional_json_string(m, "provider"),
                "project_id": optional_json_string(m, "project_id"),
                "slug": optional_json_string(m, "slug"),
                "title": optional_json_string(m, "title"),
                "describe": optional_json_string(m, "describe")
                    .or_else(|| optional_json_string(m, "description")),
                "matched_query": optional_json_string(m, "matched_query"),
            })
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

async fn generate_customization_queries(
    openai: &OpenAiClient,
    user_prompt: &str,
    base: &SelectedBasePack,
    target: &TargetCompatibility,
    existing_mods: &[serde_json::Value],
) -> Result<GeneratedModSearchPlan> {
    let response = openai
        .complete(&OpenAiTextRequest {
            instructions: vec![
                MAIN_AGENT_SYSTEM_PROMPT.to_string(),
                CUSTOMIZATION_QUERY_PROMPT.to_string(),
            ],
            input: serde_json::json!({
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
                "existing_extra_mods": existing_mod_summaries(existing_mods),
            })
            .to_string(),
            max_output_tokens: Some(360),
            temperature: Some(0.1),
            text_format: Some(mod_query_text_format()),
        })
        .await?;
    let plan = parse_mod_query_response(&response.text)?;
    Ok(GeneratedModSearchPlan {
        model: response.model,
        queries: plan.queries,
        retain_existing_mods: plan.retain_existing_mods,
        remove_existing_mod_ids: plan.remove_existing_mod_ids,
    })
}

async fn generate_customization_self_critique(
    openai: &OpenAiClient,
    user_prompt: &str,
    base: &SelectedBasePack,
    target: &TargetCompatibility,
    extra_mods: &[serde_json::Value],
    validation: &serde_json::Value,
) -> Result<GeneratedCustomizationCritique> {
    let response = openai
        .complete(&OpenAiTextRequest {
            instructions: vec![
                MAIN_AGENT_SYSTEM_PROMPT.to_string(),
                CUSTOMIZATION_SELF_CRITIQUE_PROMPT.to_string(),
            ],
            input: serde_json::json!({
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
                "tool_validated_extra_mods": existing_mod_summaries(extra_mods),
                "validation": validation,
            })
            .to_string(),
            max_output_tokens: Some(360),
            temperature: Some(0.0),
            text_format: Some(customization_critique_text_format()),
        })
        .await?;
    let critique = parse_customization_critique_response(&response.text)?;
    Ok(GeneratedCustomizationCritique {
        model: response.model,
        verdict: critique.verdict,
        remove_project_ids: critique.remove_project_ids,
        additional_queries: critique.additional_queries,
        rationale: critique.rationale,
    })
}

pub(super) async fn run_customization_planning_loop(
    openai: &OpenAiClient,
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
    let already_installed = base_modlist
        .refs
        .iter()
        .map(ModRef::key)
        .collect::<HashSet<_>>();
    let mut excluded_project_ids = HashSet::new();
    let mut additional_queries: Vec<String> = Vec::new();
    let mut last_blockers = Vec::new();

    for iteration in 1..=CUSTOMIZATION_MAX_ITERATIONS {
        let loop_prompt = if additional_queries.is_empty() && last_blockers.is_empty() {
            planning_input.to_string()
        } else {
            format!(
                "{planning_input}\n\nPrevious tool feedback:\n{}\n\nAdditional query hints: {}",
                last_blockers.join("\n"),
                additional_queries.join(", ")
            )
        };
        let generated_plan =
            generate_customization_queries(openai, &loop_prompt, base, target, existing_mods)
                .await?;
        run.push_trace(format!(
            "llm generated customization search plan via {}",
            generated_plan.model
        ));

        let mut queries = generated_plan.queries.clone();
        queries.extend(additional_queries.clone());
        queries = dedupe_queries(queries);

        let started = Instant::now();
        let mut mods = search_customization_mods(&queries, target).await?;
        mods.retain(|candidate| !excluded_project_ids.contains(&candidate.hit.id));
        run.push_tool_trace(AgentToolTrace {
            event: "customization loop search_mods".into(),
            stage: AgentPhase::CustomizationPlanning,
            iteration,
            tool: "search_mods".into(),
            input: serde_json::json!({
                "queries": queries,
                "target": {
                    "minecraft_version": target.minecraft_version.clone(),
                    "loader": target.loader.clone(),
                },
                "excluded_project_ids": excluded_project_ids.iter().cloned().collect::<Vec<_>>(),
            }),
            output: serde_json::json!({ "count": mods.len() }),
            duration_ms: started.elapsed().as_millis(),
            status: "ok".into(),
        });

        let mut roots = mods.iter().map(mod_ref_from_candidate).collect::<Vec<_>>();
        if generated_plan.retain_existing_mods {
            let removed = generated_plan
                .remove_existing_mod_ids
                .iter()
                .map(|s| s.as_str())
                .collect::<HashSet<_>>();
            roots.extend(
                existing_mods
                    .iter()
                    .filter_map(mod_ref_from_payload)
                    .filter(|r| !removed.contains(r.project_id.as_str())),
            );
        }
        roots = dedupe_mod_refs(roots);

        let started = Instant::now();
        let dep_resolution =
            resolve_dependencies(&registry, &roots, mc_version, loader, &already_installed).await?;
        let dependency_validation = dependency_resolution_payload(&dep_resolution);
        run.push_tool_trace(AgentToolTrace {
            event: "customization loop resolve_dependencies".into(),
            stage: AgentPhase::CustomizationPlanning,
            iteration,
            tool: "resolve_dependencies".into(),
            input: serde_json::json!({
                "roots": mod_ref_payloads(&roots),
                "already_installed": mod_ref_payloads(&base_modlist.refs),
                "mc_version": mc_version,
                "loader": loader,
            }),
            output: dependency_validation.clone(),
            duration_ms: started.elapsed().as_millis(),
            status: "ok".into(),
        });

        let started = Instant::now();
        let blockers = customization_blockers(&dep_resolution);
        run.push_tool_trace(AgentToolTrace {
            event: "customization loop detect_conflicts".into(),
            stage: AgentPhase::CustomizationPlanning,
            iteration,
            tool: "detect_conflicts".into(),
            input: serde_json::json!({ "resolved": dependency_validation }),
            output: serde_json::json!({ "blockers": blockers }),
            duration_ms: started.elapsed().as_millis(),
            status: if blockers.is_empty() { "ok" } else { "blocked" }.into(),
        });

        if !blockers.is_empty() {
            last_blockers = blockers
                .iter()
                .filter_map(|b| {
                    b.get("reason")
                        .and_then(|v| v.as_str())
                        .map(ToOwned::to_owned)
                })
                .collect();
            for blocker in &blockers {
                if let Some(project_id) = blocker.get("project_id").and_then(|v| v.as_str()) {
                    excluded_project_ids.insert(project_id.to_string());
                }
            }
            continue;
        }

        let root_keys = roots.iter().map(ModRef::key).collect::<HashSet<_>>();
        let root_hits = mods
            .iter()
            .map(|candidate| (mod_ref_from_candidate(candidate).key(), candidate))
            .collect::<HashMap<_, _>>();
        let extra_mods = dep_resolution
            .to_install
            .iter()
            .map(|resolved| resolved_file_mod_payload(resolved, &root_keys, &root_hits))
            .collect::<Vec<_>>();
        let validation = serde_json::json!({
            "status": "validated",
            "base_modlist": {
                "source_format": base_modlist.source_format.clone(),
                "mod_refs": mod_ref_payloads(&base_modlist.refs),
                "fetch_count": base_modlist.fetch_count,
            },
            "dependency_resolution": dependency_resolution_payload(&dep_resolution),
            "auto_added_dependencies": extra_mods
                .iter()
                .filter(|m| m.get("auto_added").and_then(|v| v.as_bool()).unwrap_or(false))
                .cloned()
                .collect::<Vec<_>>(),
            "iterations": iteration,
        });

        let started = Instant::now();
        let critique = generate_customization_self_critique(
            openai,
            planning_input,
            base,
            target,
            &extra_mods,
            &validation,
        )
        .await?;
        run.push_tool_trace(AgentToolTrace {
            event: "customization loop self_critique".into(),
            stage: AgentPhase::CustomizationPlanning,
            iteration,
            tool: "self_critique".into(),
            input: serde_json::json!({
                "extra_mods": existing_mod_summaries(&extra_mods),
                "validation": validation.clone(),
            }),
            output: serde_json::json!({
                "model": critique.model.clone(),
                "verdict": match critique.verdict {
                    CustomizationCritiqueVerdict::Pass => "pass",
                    CustomizationCritiqueVerdict::Revise => "revise",
                },
                "remove_project_ids": critique.remove_project_ids.clone(),
                "additional_queries": critique.additional_queries.clone(),
                "rationale": critique.rationale.clone(),
            }),
            duration_ms: started.elapsed().as_millis(),
            status: match critique.verdict {
                CustomizationCritiqueVerdict::Pass => "ok",
                CustomizationCritiqueVerdict::Revise => "revise",
            }
            .into(),
        });

        if critique.verdict == CustomizationCritiqueVerdict::Pass {
            return Ok(CustomizationPlanningResult::Validated(
                ValidatedCustomizationPlan {
                    extra_mods,
                    validation,
                },
            ));
        }
        excluded_project_ids.extend(critique.remove_project_ids);
        additional_queries = critique.additional_queries;
    }

    Ok(CustomizationPlanningResult::Blocked(
        CustomizationPlanningBlocked {
            reason: "customization planning reached max iterations without a validated plan"
                .to_string(),
            replan_phase: AgentPhase::ConfirmCustomizationApproval,
            details: serde_json::json!({
                "max_iterations": CUSTOMIZATION_MAX_ITERATIONS,
                "last_blockers": last_blockers,
            }),
        },
    ))
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

    for text in queries {
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

fn mod_ref_from_candidate(candidate: &ModCandidate) -> ModRef {
    ModRef::new(candidate.provider, candidate.hit.id.clone())
}

fn mod_ref_from_payload(value: &serde_json::Value) -> Option<ModRef> {
    let provider = match optional_json_string(value, "provider").as_deref() {
        Some("modrinth") => ProviderId::Modrinth,
        Some("curseforge") => ProviderId::CurseForge,
        _ => return None,
    };
    let project_id = optional_json_string(value, "project_id")?;
    Some(ModRef::new(provider, project_id))
}

fn dedupe_mod_refs(refs: Vec<ModRef>) -> Vec<ModRef> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for r in refs {
        if seen.insert(r.key()) {
            out.push(r);
        }
    }
    out
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
