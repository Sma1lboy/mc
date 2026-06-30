use super::*;

pub(super) async fn generate_restriction_update(
    openai: &OpenAiClient,
    original_user_prompt: &str,
    current: &BuildRestrictions,
    user_message: &str,
    source: BuildRestrictionChangeSource,
) -> Result<GeneratedRestrictionUpdate> {
    let input = restriction_update_request_payload(
        original_user_prompt,
        current,
        user_message,
        source.clone(),
        None,
        None,
    )
    .to_string();
    let response = openai
        .complete(&OpenAiTextRequest {
            instructions: vec![
                MAIN_AGENT_SYSTEM_PROMPT.to_string(),
                REQUIREMENT_NORMALIZATION_PROMPT.to_string(),
            ],
            input,
            max_output_tokens: Some(260),
            temperature: Some(0.0),
            text_format: Some(requirement_text_format()),
        })
        .await?;
    let mut model = response.model.clone();
    let input = match parse_restriction_update_response(&response.text) {
        Ok(input) => input,
        Err(first_err) => {
            let retry_input = restriction_update_request_payload(
                original_user_prompt,
                current,
                user_message,
                source,
                Some(first_err.to_string()),
                Some(response.text.clone()),
            )
            .to_string();
            let retry = openai
                .complete(&OpenAiTextRequest {
                    instructions: vec![
                        MAIN_AGENT_SYSTEM_PROMPT.to_string(),
                        REQUIREMENT_NORMALIZATION_PROMPT.to_string(),
                        REQUIREMENT_NORMALIZATION_RETRY_PROMPT.to_string(),
                    ],
                    input: retry_input,
                    max_output_tokens: Some(260),
                    temperature: Some(0.0),
                    text_format: Some(requirement_text_format()),
                })
                .await?;
            model = retry.model.clone();
            parse_restriction_update_response(&retry.text).map_err(|second_err| {
                CoreError::other(format!(
                    "could not parse restriction tool schema output after retry: {second_err}; first error: {first_err}"
                ))
            })?
        }
    };
    Ok(GeneratedRestrictionUpdate { model, input })
}

pub(super) fn restriction_update_request_payload(
    original_user_prompt: &str,
    current: &BuildRestrictions,
    user_message: &str,
    source: BuildRestrictionChangeSource,
    schema_violation: Option<String>,
    previous_output: Option<String>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "tool": UPDATE_BUILD_RESTRICTIONS_TOOL,
        "base_revision": current.revision,
        "source": source,
        "original_user_prompt": original_user_prompt,
        "current_restrictions": current.llm_view(),
        "latest_user_message": user_message,
    });
    if let Some(obj) = payload.as_object_mut() {
        if let Some(schema_violation) = schema_violation {
            obj.insert(
                "schema_violation".to_string(),
                serde_json::json!(schema_violation),
            );
        }
        if let Some(previous_output) = previous_output {
            obj.insert(
                "previous_output".to_string(),
                serde_json::json!(previous_output),
            );
        }
    }
    payload
}

pub(super) fn parse_restriction_update_response(
    text: &str,
) -> Result<UpdateBuildRestrictionsInput> {
    let value = serde_json::from_str::<serde_json::Value>(text.trim()).map_err(|err| {
        CoreError::other(format!(
            "could not parse single restriction tool schema object: {err}: {text}"
        ))
    })?;
    let base_revision = value
        .get("base_revision")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| CoreError::other("restriction tool output missing base_revision"))?;
    let patch_value = value
        .get("patch")
        .ok_or_else(|| CoreError::other("restriction tool output missing patch"))?;
    let minecraft_version = patch_value
        .get("minecraft_version")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| is_minecraft_version(s))
        .map(ToOwned::to_owned);
    let minecraft_version_requirement = patch_value
        .get("minecraft_version_requirement")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| minecraft_version.clone());
    let loader = patch_value
        .get("loader")
        .and_then(|v| v.as_str())
        .and_then(normalize_loader);
    let feature_tags = patch_value
        .get("feature_tags")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .take(8)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let notes = patch_value
        .get("notes")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    Ok(UpdateBuildRestrictionsInput {
        base_revision,
        patch: BuildRestrictionPatch {
            minecraft_version,
            minecraft_version_requirement,
            loader,
            feature_tags: dedupe_queries(feature_tags),
            notes,
        },
    })
}

pub(super) fn update_build_restrictions(
    current: Option<BuildRestrictions>,
    input: UpdateBuildRestrictionsInput,
    source: BuildRestrictionChangeSource,
    summary: impl Into<String>,
) -> Result<UpdateBuildRestrictionsOutput> {
    let mut restrictions = current.unwrap_or_default();
    if input.base_revision != restrictions.revision {
        return Err(CoreError::other(format!(
            "{UPDATE_BUILD_RESTRICTIONS_TOOL} revision mismatch: expected {}, got {}",
            restrictions.revision, input.base_revision
        )));
    }

    let mut warnings = Vec::new();
    let minecraft_version = input
        .patch
        .minecraft_version
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| {
            if is_minecraft_version(s) {
                Some(s.to_string())
            } else {
                warnings.push(format!("ignored invalid minecraft_version: {s}"));
                None
            }
        });
    let minecraft_version_requirement = input
        .patch
        .minecraft_version_requirement
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| minecraft_version.clone());
    let loader = input.patch.loader.as_deref().and_then(normalize_loader);
    if input.patch.loader.is_some() && loader.is_none() {
        warnings.push("ignored unsupported loader".to_string());
    }
    let feature_tags = normalize_feature_tags(input.patch.feature_tags);
    let notes = input
        .patch
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let patch = BuildRestrictionPatch {
        minecraft_version,
        minecraft_version_requirement,
        loader,
        feature_tags,
        notes,
    };

    restrictions.minecraft_version = patch.minecraft_version.clone();
    restrictions.minecraft_version_requirement = patch.minecraft_version_requirement.clone();
    restrictions.loader = patch.loader.clone();
    restrictions.feature_tags = patch.feature_tags.clone();
    restrictions.notes = patch.notes.clone();
    restrictions.revision += 1;
    restrictions.history.push(BuildRestrictionChange {
        revision: restrictions.revision,
        source,
        patch,
        summary: summary.into(),
    });

    Ok(UpdateBuildRestrictionsOutput {
        missing_fields: missing_restriction_fields(&restrictions),
        restrictions,
        warnings,
    })
}

fn normalize_feature_tags(tags: Vec<String>) -> Vec<String> {
    dedupe_queries(
        tags.into_iter()
            .map(|tag| tag.trim().to_ascii_lowercase())
            .filter(|tag| !tag.is_empty())
            .take(8)
            .collect(),
    )
}

pub(super) fn restriction_target_changed(
    before: &BuildRestrictions,
    after: &BuildRestrictions,
) -> bool {
    before.minecraft_version != after.minecraft_version
        || before.minecraft_version_requirement != after.minecraft_version_requirement
        || before.loader != after.loader
}

pub(super) fn changed_restriction_field(
    before: &BuildRestrictions,
    after: &BuildRestrictions,
) -> Option<ChangedField> {
    if before.minecraft_version != after.minecraft_version {
        Some(ChangedField::MinecraftVersion)
    } else if before.loader != after.loader {
        Some(ChangedField::Loader)
    } else if before.minecraft_version_requirement != after.minecraft_version_requirement {
        Some(ChangedField::VersionRequirement)
    } else if before.feature_tags != after.feature_tags {
        Some(ChangedField::ContentPreference)
    } else if before.notes != after.notes {
        Some(ChangedField::SearchPreference)
    } else {
        None
    }
}

fn invalidates_for_changed_field(changed: ChangedField) -> Vec<PlanArtifact> {
    match changed {
        ChangedField::MinecraftVersion
        | ChangedField::Loader
        | ChangedField::VersionRequirement => {
            vec![
                PlanArtifact::BasePack,
                PlanArtifact::ExtraMods,
                PlanArtifact::ApprovedBuild,
                PlanArtifact::ExecutionMetadata,
            ]
        }
        ChangedField::ContentPreference => vec![
            PlanArtifact::ExtraMods,
            PlanArtifact::ApprovedBuild,
            PlanArtifact::ExecutionMetadata,
        ],
        ChangedField::SearchPreference => vec![
            PlanArtifact::BasePack,
            PlanArtifact::ExtraMods,
            PlanArtifact::ApprovedBuild,
            PlanArtifact::ExecutionMetadata,
        ],
        ChangedField::BasePack => vec![
            PlanArtifact::ExtraMods,
            PlanArtifact::ApprovedBuild,
            PlanArtifact::ExecutionMetadata,
        ],
    }
}

fn target_phase_for_changed_field(changed: ChangedField, from_phase: &AgentPhase) -> AgentPhase {
    match changed {
        ChangedField::MinecraftVersion
        | ChangedField::Loader
        | ChangedField::VersionRequirement => AgentPhase::ConfigureRequirementsApproval,
        ChangedField::ContentPreference => match from_phase {
            AgentPhase::ConfigureRequirementsApproval => AgentPhase::ConfigureRequirementsApproval,
            AgentPhase::ChooseBasePackApproval | AgentPhase::BasePackSearch => {
                AgentPhase::ChooseBasePackApproval
            }
            _ => AgentPhase::ConfirmCustomizationApproval,
        },
        ChangedField::SearchPreference => AgentPhase::ChooseBasePackApproval,
        ChangedField::BasePack => AgentPhase::ChooseBasePackApproval,
    }
}

pub(super) fn invalidate_downstream(
    run: &mut AgentRunSnapshot,
    changed: ChangedField,
    reason: impl Into<String>,
    from_phase: AgentPhase,
    restriction_patch: Option<BuildRestrictionPatch>,
) {
    let reason = reason.into();
    let invalidates = invalidates_for_changed_field(changed);
    let target_phase = target_phase_for_changed_field(changed, &from_phase);

    if invalidates.contains(&PlanArtifact::ApprovedBuild) {
        run.approved_build = None;
    }
    if invalidates.contains(&PlanArtifact::ExecutionMetadata) {
        run.execution = None;
    }

    let duplicate = run.replans.iter().any(|existing| {
        existing.reason == reason
            && existing.from_phase == from_phase
            && existing.target_phase == target_phase
            && existing.restriction_patch == restriction_patch
            && existing.invalidates == invalidates
    });
    if !duplicate {
        run.push_replan(PlanReplanRequest {
            id: crate::agent::state::new_id("replan"),
            reason,
            from_phase,
            target_phase,
            restriction_patch,
            invalidates,
        });
    }
}

pub(super) fn apply_requirements_replan(
    mut run: AgentRunSnapshot,
    output: UpdateBuildRestrictionsOutput,
    reason: impl Into<String>,
    from_phase: AgentPhase,
) -> AgentRunSnapshot {
    let reason = reason.into();
    let restriction_patch = output
        .restrictions
        .history
        .last()
        .map(|change| change.patch.clone());

    run.restrictions = Some(output.restrictions.clone());
    invalidate_downstream(
        &mut run,
        restriction_patch
            .as_ref()
            .map(|patch| {
                if patch.minecraft_version.is_some() {
                    ChangedField::MinecraftVersion
                } else if patch.loader.is_some() {
                    ChangedField::Loader
                } else {
                    ChangedField::VersionRequirement
                }
            })
            .unwrap_or(ChangedField::VersionRequirement),
        reason.clone(),
        from_phase,
        restriction_patch,
    );
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfigureRequirementsApproval;
    run.pending_approval = Some(requirements_approval(&run.user_prompt, &output));
    run.plan = Some(requirements_plan(&run.user_prompt, &output));
    run.push_message(
        AgentMessageKind::Assistant,
        format!("需求规格需要重新确认: {reason}"),
    );
    run.push_trace("plan replan requested; invalidated downstream artifacts");
    run
}

pub(super) async fn continue_after_requirements_confirmation(
    openai: &OpenAiClient,
    mut run: AgentRunSnapshot,
    selected: ApprovalOption,
) -> Result<AgentRunSnapshot> {
    let restrictions = run
        .restrictions
        .clone()
        .or_else(|| {
            selected
                .payload
                .as_ref()
                .and_then(restrictions_from_requirement_payload)
        })
        .ok_or_else(|| CoreError::other("requirements approval has no restrictions state"))?;

    run.push_message(
        AgentMessageKind::User,
        format!("确认整合包规格: {}", requirement_label(&restrictions)),
    );
    run.restrictions = Some(restrictions);
    run.approved_build = None;
    run.execution = None;
    run.pending_approval = None;
    run.push_trace("approved normalized build requirements");
    continue_to_base_pack_search(openai, run).await
}

pub(super) async fn continue_after_requirements_feedback(
    openai: &OpenAiClient,
    mut run: AgentRunSnapshot,
    feedback: &str,
) -> Result<AgentRunSnapshot> {
    run.push_message(
        AgentMessageKind::User,
        format!("修改整合包规格: {feedback}"),
    );
    run.pending_approval = None;
    run.push_trace("received build requirement feedback; updating restrictions");

    let current = run.restrictions.clone().unwrap_or_default();
    let generated = generate_restriction_update(
        openai,
        &run.user_prompt,
        &current,
        feedback,
        BuildRestrictionChangeSource::UserRevise,
    )
    .await?;
    let output = update_build_restrictions(
        Some(current.clone()),
        generated.input,
        BuildRestrictionChangeSource::UserRevise,
        feedback,
    )?;
    if let Some(changed) = changed_restriction_field(&current, &output.restrictions) {
        let patch = output
            .restrictions
            .history
            .last()
            .map(|change| change.patch.clone());
        invalidate_downstream(
            &mut run,
            changed,
            format!("requirements revised: {feedback}"),
            AgentPhase::ConfigureRequirementsApproval,
            patch,
        );
    }
    run.push_trace(format!(
        "llm generated build restriction update via {}",
        generated.model
    ));
    run.push_message(
        AgentMessageKind::Assistant,
        requirement_summary_message(&output),
    );
    let approval = requirements_approval(&run.user_prompt, &output);
    run.status = AgentStatus::WaitingForUser;
    run.phase = AgentPhase::ConfigureRequirementsApproval;
    run.restrictions = Some(output.restrictions.clone());
    run.pending_approval = Some(approval);
    run.plan = Some(requirements_plan(&run.user_prompt, &output));
    run.push_trace("paused at updated build requirements approval gate");
    Ok(run)
}

pub(super) async fn maybe_replan_requirements_from_feedback(
    openai: &OpenAiClient,
    mut run: AgentRunSnapshot,
    feedback: &str,
) -> Result<Option<AgentRunSnapshot>> {
    let current = run.restrictions.clone().unwrap_or_default();
    let generated = generate_restriction_update(
        openai,
        &run.user_prompt,
        &current,
        feedback,
        BuildRestrictionChangeSource::UserRevise,
    )
    .await?;
    let output = update_build_restrictions(
        Some(current.clone()),
        generated.input,
        BuildRestrictionChangeSource::UserRevise,
        feedback,
    )?;
    if !restriction_target_changed(&current, &output.restrictions) {
        return Ok(None);
    }

    let from_phase = run.phase.clone();
    run.push_message(AgentMessageKind::User, format!("修改定制需求: {feedback}"));
    let mut replanned = apply_requirements_replan(
        run,
        output,
        format!("用户在定制阶段修改了 version/loader: {feedback}"),
        from_phase,
    );
    replanned.push_trace(format!(
        "llm generated build restriction replan via {}",
        generated.model
    ));
    Ok(Some(replanned))
}
