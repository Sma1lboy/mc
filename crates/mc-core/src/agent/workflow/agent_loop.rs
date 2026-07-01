use super::*;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ModpackAgentAction {
    ToolCall {
        tool: String,
        #[serde(default)]
        args: serde_json::Value,
        #[serde(default)]
        rationale: Option<String>,
    },
    RequestApproval {
        approval_kind: ModpackAgentApprovalKind,
        #[serde(default)]
        rationale: Option<String>,
    },
    RequestInput {
        input_kind: ModpackAgentInputKind,
        #[serde(default)]
        args: serde_json::Value,
        #[serde(default)]
        rationale: Option<String>,
    },
    Final {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ModpackAgentApprovalKind {
    ConfigureRequirements,
    ChooseBasePack,
    ConfirmCustomization,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::agent::workflow) enum ModpackAgentInputKind {
    SelectMinecraftVersion,
}

fn parse_modpack_agent_action(model_text: &str) -> Result<ModpackAgentAction> {
    let value = llm_io::parse_first_json_object(model_text).ok_or_else(|| {
        CoreError::other(format!(
            "could not parse modpack agent action JSON from model output: {model_text}"
        ))
    })?;
    serde_json::from_value(value)
        .map_err(|err| CoreError::other(format!("invalid modpack agent action: {err}")))
}

pub(super) fn apply_extracted_modpack_goals(
    mut run: AgentRunSnapshot,
    args: serde_json::Value,
    turn: u32,
) -> Result<AgentRunSnapshot> {
    let goals = args
        .get("goals")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let goals = dedupe_queries(goals)
        .into_iter()
        .take(8)
        .collect::<Vec<_>>();
    if goals.is_empty() {
        return Err(CoreError::other(
            "extract_modpack_goals requires non-empty goals[]",
        ));
    }

    let current = run.restrictions.clone().unwrap_or_default();
    let input = UpdateBuildRestrictionsInput {
        base_revision: current.revision,
        patch: BuildRestrictionPatch {
            minecraft_version: current.minecraft_version.clone(),
            minecraft_version_requirement: current.minecraft_version_requirement.clone(),
            loader: current.loader.clone(),
            feature_tags: goals.clone(),
            notes: current.notes.clone(),
        },
    };
    let source = if current.revision == 0 {
        BuildRestrictionChangeSource::InitialPrompt
    } else {
        BuildRestrictionChangeSource::UserRevise
    };
    let output = update_build_restrictions(Some(current), input, source, "agent goals tool call")?;
    run.restrictions = Some(output.restrictions.clone());
    set_agent_memory(
        &mut run,
        "modpack_goals",
        serde_json::Value::Array(
            output
                .restrictions
                .feature_tags
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );
    let requirements_output = serde_json::to_value(&output).map_err(|err| {
        CoreError::other(format!(
            "could not serialize extract_modpack_goals output: {err}"
        ))
    })?;
    set_agent_memory(&mut run, "requirements_output", requirements_output);
    let tool_output = serde_json::json!({
        "goals": output.restrictions.feature_tags,
    });
    run.push_message(
        AgentMessageKind::Tool,
        format!(
            "extracted modpack goals: {}",
            tool_output
                .get("goals")
                .and_then(|value| value.as_array())
                .map(|goals| {
                    goals
                        .iter()
                        .filter_map(|goal| goal.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        ),
    );
    run.push_tool_trace(AgentToolTrace {
        event: "agent tool extract_modpack_goals".into(),
        stage: AgentPhase::IntentRouting,
        iteration: turn,
        tool: EXTRACT_MODPACK_GOALS_TOOL.into(),
        input: serde_json::json!({ "goals": goals }),
        output: tool_output,
        duration_ms: 0,
        status: "ok".into(),
    });
    Ok(run)
}

pub(super) struct ModpackBuildAgent {
    llm: AgentLlmClient,
}

impl ModpackBuildAgent {
    pub fn new(llm: AgentLlmClient) -> Self {
        Self { llm }
    }

    pub async fn start(&self, user_prompt: &str) -> Result<AgentRunSnapshot> {
        let mut run = begin_modpack_build_react_run(user_prompt);
        run.push_trace("started modpack_build tool loop");
        self.run_tool_loop(run).await
    }

    /// Continue from a saved waiting snapshot by applying the user's approval
    /// decision, then returning control to the model-selected tool loop when
    /// more planning is needed.
    pub async fn continue_run(
        &self,
        run: AgentRunSnapshot,
        decision: UserDecision,
    ) -> Result<AgentRunSnapshot> {
        let approval = pending_approval(&run)?;
        validate_approval_id(&approval, &decision)?;
        validate_user_decision_shape(&decision)?;

        if decision.kind == UserDecisionKind::Cancel {
            let mut run = run;
            run.complete(AgentPhase::Completed);
            run.push_trace("user cancelled agent run");
            return Ok(run);
        }

        if approval.kind == ApprovalKind::ConfirmCustomization
            && decision.kind == UserDecisionKind::Approve
        {
            let selected = selected_approval_option(&approval, &decision, "confirm_customization")?;
            if selected.id == "confirm:recommended_customization" {
                return continue_after_customization_confirmation(run, selected);
            }
        }

        let run = apply_modpack_build_user_decision(run, &approval, decision)?;
        self.run_tool_loop(run).await
    }

    pub(in crate::agent::workflow) async fn run_tool_loop(
        &self,
        mut run: AgentRunSnapshot,
    ) -> Result<AgentRunSnapshot> {
        for turn in 1..=MODPACK_AGENT_MAX_TURNS {
            if run.status != AgentStatus::Running {
                return Ok(run);
            }

            let output = self
                .llm
                .prompt_text(
                    &[
                        MAIN_AGENT_SYSTEM_PROMPT,
                        modpack_build_react_prompt(),
                        MODPACK_BUILD_TOOL_LOOP_PROMPT,
                    ],
                    modpack_agent_context(&run, turn).to_string(),
                    700,
                    0.1,
                )
                .await?;
            let action = parse_modpack_agent_action(&output)?;
            match action {
                ModpackAgentAction::ToolCall {
                    tool,
                    args,
                    rationale,
                } => {
                    if let Some(rationale) = rationale.filter(|s| !s.trim().is_empty()) {
                        run.push_trace(format!("agent selected tool {tool}: {rationale}"));
                    }
                    run = self.execute_modpack_tool(run, &tool, args, turn).await?;
                }
                ModpackAgentAction::RequestApproval {
                    approval_kind,
                    rationale,
                } => {
                    if let Some(rationale) = rationale.filter(|s| !s.trim().is_empty()) {
                        run.push_trace(format!("agent requested approval: {rationale}"));
                    }
                    return request_modpack_agent_approval(run, approval_kind);
                }
                ModpackAgentAction::RequestInput {
                    input_kind,
                    args,
                    rationale,
                } => {
                    if let Some(rationale) = rationale.filter(|s| !s.trim().is_empty()) {
                        run.push_trace(format!("agent requested input: {rationale}"));
                    }
                    return request_modpack_agent_input(run, input_kind, args);
                }
                ModpackAgentAction::Final { message } => {
                    run.push_message(AgentMessageKind::Assistant, message);
                    run.complete(AgentPhase::Completed);
                    return Ok(run);
                }
            }
        }

        Err(CoreError::other(format!(
            "modpack_build agent exceeded {MODPACK_AGENT_MAX_TURNS} tool-loop turns"
        )))
    }

    async fn execute_modpack_tool(
        &self,
        mut run: AgentRunSnapshot,
        tool: &str,
        args: serde_json::Value,
        turn: u32,
    ) -> Result<AgentRunSnapshot> {
        match tool {
            UPDATE_BUILD_RESTRICTIONS_TOOL => {
                let input = serde_json::from_value::<UpdateBuildRestrictionsInput>(args).map_err(
                    |err| {
                        CoreError::other(format!(
                            "invalid update_build_restrictions tool args: {err}"
                        ))
                    },
                )?;
                let source = if run.restrictions.is_some() {
                    BuildRestrictionChangeSource::UserRevise
                } else {
                    BuildRestrictionChangeSource::InitialPrompt
                };
                let current = run.restrictions.clone().unwrap_or_default();
                let output =
                    update_build_restrictions(Some(current), input, source, "agent tool call")?;
                run.restrictions = Some(output.restrictions.clone());
                run.push_message(
                    AgentMessageKind::Assistant,
                    requirement_summary_message(&output),
                );
                run.push_tool_trace(AgentToolTrace {
                    event: "agent tool update_build_restrictions".into(),
                    stage: AgentPhase::IntentRouting,
                    iteration: turn,
                    tool: UPDATE_BUILD_RESTRICTIONS_TOOL.into(),
                    input: serde_json::json!({ "base_revision": output.restrictions.revision.saturating_sub(1) }),
                    output: serde_json::to_value(&output).unwrap_or_else(|_| serde_json::json!({})),
                    duration_ms: 0,
                    status: "ok".into(),
                });
                let output_value = serde_json::to_value(&output).map_err(|err| {
                    CoreError::other(format!(
                        "could not serialize update_build_restrictions output: {err}"
                    ))
                })?;
                set_agent_memory(&mut run, "requirements_output", output_value);
                Ok(run)
            }
            EXTRACT_MODPACK_GOALS_TOOL => apply_extracted_modpack_goals(run, args, turn),
            "modpack_search" => {
                let queries = args
                    .get("queries")
                    .and_then(|v| v.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let queries = dedupe_queries(queries)
                    .into_iter()
                    .take(6)
                    .collect::<Vec<_>>();
                if queries.is_empty() {
                    return Err(CoreError::other("modpack_search requires queries[]"));
                }
                run.phase = AgentPhase::BasePackSearch;
                let requested =
                    requested_compatibility_from_restrictions(run.restrictions.as_ref());
                let candidates = run_base_pack_search_loop(&mut run, &queries, &requested).await?;
                let plan = selection_plan(&planning_context_input(&run), &queries, &candidates);
                let approval = base_pack_selection_approval(&candidates, plan.clone());
                run.plan = Some(plan);
                run.push_message(
                    AgentMessageKind::Tool,
                    format!("modpack_search returned {} candidates", candidates.len()),
                );
                let approval_value = serde_json::to_value(&approval).map_err(|err| {
                    CoreError::other(format!("could not serialize base-pack approval: {err}"))
                })?;
                set_agent_memory(&mut run, "choose_base_pack_approval", approval_value);
                Ok(run)
            }
            "plan_customization" | "mod_search" | "mod_get_detail" | "compatibility_check" => {
                if tool != "plan_customization" {
                    run.push_trace(format!(
                        "agent selected customization tool {tool}; running customization planning pipeline"
                    ));
                }
                let selected = selected_base_pack_from_memory(&run)?;
                plan_customization_after_base_pack_choice(&self.llm, run, selected).await
            }
            other => Err(CoreError::other(format!(
                "unsupported modpack_build tool call: {other}"
            ))),
        }
    }
}

/// Apply a human approval decision to the snapshot without selecting the next
/// business step. The next step is chosen by the modpack agent tool loop.
pub fn apply_modpack_build_user_decision(
    mut run: AgentRunSnapshot,
    approval: &ApprovalRequest,
    decision: UserDecision,
) -> Result<AgentRunSnapshot> {
    validate_approval_id(&approval, &decision)?;
    validate_user_decision_shape(&decision)?;

    if decision.kind == UserDecisionKind::Cancel {
        run.complete(AgentPhase::Completed);
        run.push_trace("user cancelled agent run");
        return Ok(run);
    }

    match (&approval.kind, &decision.kind) {
        (_, UserDecisionKind::Revise) => {
            let feedback = decision
                .message
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| CoreError::other("revise decision requires message"))?
                .to_string();
            run.push_message(
                AgentMessageKind::User,
                format!("Approval feedback: {feedback}"),
            );
            set_agent_memory(
                &mut run,
                "latest_feedback",
                serde_json::json!({
                    "approval_kind": approval.kind,
                    "message": feedback,
                }),
            );
            run.clear_user_interrupt();
            Ok(run)
        }
        (ApprovalKind::ConfigureRequirements, UserDecisionKind::Approve) => {
            let selected = selected_approval_option(approval, &decision, "configure_requirements")?;
            if let Some(payload) = selected.payload.as_ref() {
                if let Some(restrictions) = restrictions_from_requirement_payload(payload) {
                    run.restrictions = Some(restrictions);
                }
            }
            run.push_message(AgentMessageKind::User, "Confirmed modpack requirements");
            set_agent_memory(
                &mut run,
                "requirements_confirmed",
                serde_json::json!({ "approval_id": approval.id }),
            );
            run.clear_user_interrupt();
            Ok(run)
        }
        (
            ApprovalKind::ChooseBasePack | ApprovalKind::ConfirmScratchFallback,
            UserDecisionKind::Approve,
        ) => {
            let selected = selected_approval_option(&approval, &decision, "base-pack approval")?;
            run.push_message(
                AgentMessageKind::User,
                format!("Selected base modpack option: {}", selected.label),
            );
            set_agent_memory(
                &mut run,
                "selected_base_pack_option",
                serde_json::to_value(&selected).unwrap_or_else(|_| serde_json::json!({})),
            );
            run.clear_user_interrupt();
            Ok(run)
        }
        (ApprovalKind::ConfirmCustomization, UserDecisionKind::Approve) => {
            let selected = selected_approval_option(&approval, &decision, "confirm_customization")?;
            if selected.id == "back:choose_base_pack" {
                run.push_message(
                    AgentMessageKind::User,
                    "Returned to base-pack selection from customization",
                );
                set_agent_memory(
                    &mut run,
                    "latest_feedback",
                    serde_json::json!({
                        "approval_kind": approval.kind,
                        "message": "return to base-pack selection",
                    }),
                );
                run.clear_user_interrupt();
                return Ok(run);
            }
            set_agent_memory(
                &mut run,
                "selected_customization_option",
                serde_json::to_value(&selected).unwrap_or_else(|_| serde_json::json!({})),
            );
            run.clear_user_interrupt();
            Ok(run)
        }
        other => Err(CoreError::other(format!(
            "continue for approval kind {other:?} is not implemented yet"
        ))),
    }
}

pub fn apply_modpack_build_user_input(
    run: AgentRunSnapshot,
    interrupt: &AgentInterrupt,
    value: &str,
) -> Result<AgentRunSnapshot> {
    let current = run
        .pending_interrupt
        .as_ref()
        .ok_or_else(|| CoreError::other("agent session has no pending input interrupt"))?;
    if current.id != interrupt.id {
        return Err(CoreError::other(format!(
            "interrupt id mismatch: expected {}, got {}",
            current.id, interrupt.id
        )));
    }

    match interrupt.input_kind.as_ref() {
        Some(AgentInputKind::SelectMinecraftVersion) => {
            apply_minecraft_version_selection_input(run, value)
        }
        None => Err(CoreError::other(
            "pending input interrupt is missing input_kind",
        )),
    }
}

fn apply_minecraft_version_selection_input(
    mut run: AgentRunSnapshot,
    value: &str,
) -> Result<AgentRunSnapshot> {
    let selected = value.trim();
    if !is_minecraft_version(selected) {
        return Err(CoreError::other(format!(
            "select_minecraft_version requires a concrete Minecraft version, got {selected:?}"
        )));
    }

    let current = run.restrictions.clone().unwrap_or_default();
    let input = UpdateBuildRestrictionsInput {
        base_revision: current.revision,
        patch: BuildRestrictionPatch {
            minecraft_version: Some(selected.to_string()),
            minecraft_version_requirement: Some(selected.to_string()),
            loader: current.loader.clone(),
            feature_tags: current.feature_tags.clone(),
            notes: current.notes.clone(),
        },
    };
    let source = if current.revision == 0 {
        BuildRestrictionChangeSource::InitialPrompt
    } else {
        BuildRestrictionChangeSource::UserRevise
    };
    let output = update_build_restrictions(
        Some(current),
        input,
        source,
        "user selected minecraft version",
    )?;
    run.restrictions = Some(output.restrictions.clone());
    let output_value = serde_json::to_value(&output).map_err(|err| {
        CoreError::other(format!(
            "could not serialize version selection requirements output: {err}"
        ))
    })?;
    set_agent_memory(&mut run, "requirements_output", output_value);
    run.push_message(
        AgentMessageKind::User,
        format!("Selected Minecraft version: {selected}"),
    );
    run.clear_user_interrupt();
    run.push_trace("applied minecraft version selection input");
    Ok(run)
}

pub(in crate::agent::workflow) fn pending_approval(
    run: &AgentRunSnapshot,
) -> Result<ApprovalRequest> {
    run.pending_approval
        .clone()
        .ok_or_else(|| CoreError::other("agent session has no pending approval"))
}

pub(super) fn pending_user_input(run: &AgentRunSnapshot) -> Result<AgentInterrupt> {
    run.pending_interrupt
        .clone()
        .filter(|interrupt| interrupt.kind == AgentInterruptKind::UserInput)
        .ok_or_else(|| CoreError::other("agent session has no pending user input"))
}

fn modpack_agent_context(run: &AgentRunSnapshot, turn: u32) -> serde_json::Value {
    serde_json::json!({
        "turn": turn,
        "user_prompt": run.user_prompt,
        "status": run.status,
        "ui_phase_projection": run.phase,
        "restrictions": run.restrictions,
        "agent_memory": run.agent_memory,
        "approved_build": run.approved_build,
        "execution": run.execution,
        "available_tools": run.tools,
        "recent_messages": run.messages.iter().rev().take(8).cloned().collect::<Vec<_>>(),
    })
}

pub(super) fn set_agent_memory(run: &mut AgentRunSnapshot, key: &str, value: serde_json::Value) {
    if !run.agent_memory.is_object() {
        run.agent_memory = serde_json::json!({});
    }
    if let Some(obj) = run.agent_memory.as_object_mut() {
        obj.insert(key.to_string(), value);
    }
}

fn agent_memory_value<'a>(run: &'a AgentRunSnapshot, key: &str) -> Option<&'a serde_json::Value> {
    run.agent_memory.get(key)
}

fn selected_base_pack_from_memory(run: &AgentRunSnapshot) -> Result<ApprovalOption> {
    agent_memory_value(run, "selected_base_pack_option")
        .cloned()
        .ok_or_else(|| CoreError::other("plan_customization requires a selected base pack"))
        .and_then(|value| {
            serde_json::from_value::<ApprovalOption>(value).map_err(|err| {
                CoreError::other(format!("invalid selected base pack memory: {err}"))
            })
        })
}

fn request_modpack_agent_approval(
    mut run: AgentRunSnapshot,
    kind: ModpackAgentApprovalKind,
) -> Result<AgentRunSnapshot> {
    match kind {
        ModpackAgentApprovalKind::ConfigureRequirements => {
            let output = agent_memory_value(&run, "requirements_output")
                .cloned()
                .and_then(|value| {
                    serde_json::from_value::<UpdateBuildRestrictionsOutput>(value).ok()
                })
                .or_else(|| {
                    run.restrictions
                        .clone()
                        .map(|restrictions| restrictions.as_update_output(Vec::new()))
                })
                .ok_or_else(|| {
                    CoreError::other(
                        "configure_requirements approval requires update_build_restrictions output",
                    )
                })?;
            let approval = requirements_approval(&run.user_prompt, &output);
            let plan = requirements_plan(&run.user_prompt, &output);
            run.request_approval(
                AgentPhase::ConfigureRequirementsApproval,
                approval,
                Some(plan),
            );
            run.push_trace("agent paused at requirements approval interrupt");
            Ok(run)
        }
        ModpackAgentApprovalKind::ChooseBasePack => {
            let approval = agent_memory_value(&run, "choose_base_pack_approval")
                .cloned()
                .ok_or_else(|| {
                    CoreError::other("choose_base_pack approval requires modpack_search output")
                })
                .and_then(|value| {
                    serde_json::from_value::<ApprovalRequest>(value).map_err(|err| {
                        CoreError::other(format!("invalid base-pack approval memory: {err}"))
                    })
                })?;
            let plan = approval.plan.clone();
            run.request_approval(AgentPhase::ChooseBasePackApproval, approval, plan);
            run.push_trace("agent paused at base-pack approval interrupt");
            Ok(run)
        }
        ModpackAgentApprovalKind::ConfirmCustomization => {
            let approval = agent_memory_value(&run, "confirm_customization_approval")
                .cloned()
                .ok_or_else(|| {
                    CoreError::other(
                        "confirm_customization approval requires plan_customization output",
                    )
                })
                .and_then(|value| {
                    serde_json::from_value::<ApprovalRequest>(value).map_err(|err| {
                        CoreError::other(format!("invalid customization approval memory: {err}"))
                    })
                })?;
            let plan = approval.plan.clone();
            run.request_approval(AgentPhase::ConfirmCustomizationApproval, approval, plan);
            run.push_trace("agent paused at customization approval interrupt");
            Ok(run)
        }
    }
}

pub(in crate::agent::workflow) fn request_modpack_agent_input(
    run: AgentRunSnapshot,
    kind: ModpackAgentInputKind,
    args: serde_json::Value,
) -> Result<AgentRunSnapshot> {
    match kind {
        ModpackAgentInputKind::SelectMinecraftVersion => {
            request_minecraft_version_selection_input(run, args)
        }
    }
}

fn request_minecraft_version_selection_input(
    mut run: AgentRunSnapshot,
    args: serde_json::Value,
) -> Result<AgentRunSnapshot> {
    let candidates = args
        .get("candidates")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let candidates = dedupe_queries(candidates)
        .into_iter()
        .filter(|version| is_minecraft_version(version))
        .take(12)
        .collect::<Vec<_>>();
    let options = candidates
        .iter()
        .map(|version| AgentInputOption {
            id: version.clone(),
            label: version.clone(),
            description: None,
            payload: Some(serde_json::json!({ "minecraft_version": version })),
        })
        .collect::<Vec<_>>();
    let restriction_view = run.restrictions.as_ref();
    let version_request = args
        .get("version_request")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            restriction_view
                .and_then(|restrictions| restrictions.minecraft_version_requirement.clone())
        });
    let loader = args
        .get("loader")
        .and_then(|value| value.as_str())
        .and_then(normalize_loader)
        .or_else(|| restriction_view.and_then(|restrictions| restrictions.loader.clone()));
    let message = match version_request.as_deref() {
        Some(request) => format!(
            "Choose a concrete Minecraft version for the requested version range: {request}."
        ),
        None => "Choose a concrete Minecraft version for this modpack.".to_string(),
    };
    let interrupt = AgentInterrupt::user_input(
        AgentInputKind::SelectMinecraftVersion,
        "Choose Minecraft version",
        message,
        options,
        true,
        Some(serde_json::json!({
            "field": "minecraft_version",
            "version_request": version_request,
            "loader": loader,
        })),
    );
    run.request_input(AgentPhase::ConfigureRequirementsApproval, interrupt);
    run.push_trace("agent paused at minecraft version selection interrupt");
    Ok(run)
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

pub(in crate::agent::workflow) fn validate_user_decision_shape(
    decision: &UserDecision,
) -> Result<()> {
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
