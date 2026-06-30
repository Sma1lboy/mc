use super::*;

pub(super) async fn generate_restriction_update(
    llm: &AgentLlmClient,
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
    let response = llm
        .prompt_typed::<UpdateBuildRestrictionsInput>(
            &[MAIN_AGENT_SYSTEM_PROMPT, REQUIREMENT_NORMALIZATION_PROMPT],
            input,
            260,
            0.0,
        )
        .await?;
    let mut model = llm.model().to_string();
    let previous_output = format!("{response:?}");
    let input = match validate_restriction_update_input(response) {
        Ok(input) => input,
        Err(first_err) => {
            let retry_input = restriction_update_request_payload(
                original_user_prompt,
                current,
                user_message,
                source,
                Some(first_err.to_string()),
                Some(previous_output),
            )
            .to_string();
            let retry = llm
                .prompt_typed::<UpdateBuildRestrictionsInput>(
                    &[
                        MAIN_AGENT_SYSTEM_PROMPT,
                        REQUIREMENT_NORMALIZATION_PROMPT,
                        REQUIREMENT_NORMALIZATION_RETRY_PROMPT,
                    ],
                    retry_input,
                    260,
                    0.0,
                )
                .await?;
            model = llm.model().to_string();
            validate_restriction_update_retry(retry, &first_err)?
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

#[cfg(test)]
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
        .map(ToOwned::to_owned);
    let minecraft_version_requirement = patch_value
        .get("minecraft_version_requirement")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let loader = patch_value
        .get("loader")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    let feature_tags = patch_value
        .get("feature_tags")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
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
    Ok(normalize_restriction_update_input(
        UpdateBuildRestrictionsInput {
            base_revision,
            patch: BuildRestrictionPatch {
                minecraft_version,
                minecraft_version_requirement,
                loader,
                feature_tags,
                notes,
            },
        },
    ))
}

fn validate_restriction_update_input(
    input: UpdateBuildRestrictionsInput,
) -> Result<UpdateBuildRestrictionsInput> {
    Ok(normalize_restriction_update_input(input))
}

pub(super) fn validate_restriction_update_retry(
    input: UpdateBuildRestrictionsInput,
    first_err: &CoreError,
) -> Result<UpdateBuildRestrictionsInput> {
    validate_restriction_update_input(input).map_err(|second_err| {
        CoreError::other(format!(
            "could not parse restriction tool schema output after retry: {second_err}; first error: {first_err}"
        ))
    })
}

pub(super) fn normalize_restriction_update_input(
    input: UpdateBuildRestrictionsInput,
) -> UpdateBuildRestrictionsInput {
    let minecraft_version = input
        .patch
        .minecraft_version
        .as_deref()
        .map(str::trim)
        .filter(|s| is_minecraft_version(s))
        .map(ToOwned::to_owned);
    let minecraft_version_requirement = input
        .patch
        .minecraft_version_requirement
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| minecraft_version.clone());
    let loader = input.patch.loader.as_deref().and_then(normalize_loader);
    let feature_tags = input
        .patch
        .feature_tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(8)
        .collect::<Vec<_>>();
    let notes = input
        .patch
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    UpdateBuildRestrictionsInput {
        base_revision: input.base_revision,
        patch: BuildRestrictionPatch {
            minecraft_version,
            minecraft_version_requirement,
            loader,
            feature_tags: dedupe_queries(feature_tags),
            notes,
        },
    }
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
    if before.minecraft_version != after.minecraft_version || before.loader != after.loader {
        return true;
    }
    if before.minecraft_version.is_some() && after.minecraft_version.is_some() {
        return false;
    }
    before.minecraft_version_requirement != after.minecraft_version_requirement
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

pub(super) const ALL_CHANGED_FIELDS: &[ChangedField] = &[
    ChangedField::MinecraftVersion,
    ChangedField::Loader,
    ChangedField::VersionRequirement,
    ChangedField::ContentPreference,
    ChangedField::SearchPreference,
    ChangedField::BasePack,
];

const TARGET_INVALIDATES: &[PlanArtifact] = &[
    PlanArtifact::BasePack,
    PlanArtifact::ExtraMods,
    PlanArtifact::ApprovedBuild,
    PlanArtifact::ExecutionMetadata,
];
const CONTENT_INVALIDATES: &[PlanArtifact] = &[
    PlanArtifact::ExtraMods,
    PlanArtifact::ApprovedBuild,
    PlanArtifact::ExecutionMetadata,
];

#[derive(Debug, Clone, Copy)]
pub(super) struct InvalidationRule {
    pub(super) changed: ChangedField,
    pub(super) invalidates: &'static [PlanArtifact],
    target: InvalidationTarget,
}

#[derive(Debug, Clone, Copy)]
enum InvalidationTarget {
    ConfigureRequirementsApproval,
    ChooseBasePackApproval,
    ContentPreference,
}

impl InvalidationTarget {
    fn phase(self, from_phase: &AgentPhase) -> AgentPhase {
        match self {
            Self::ConfigureRequirementsApproval => AgentPhase::ConfigureRequirementsApproval,
            Self::ChooseBasePackApproval => AgentPhase::ChooseBasePackApproval,
            Self::ContentPreference => match from_phase {
                AgentPhase::ConfigureRequirementsApproval => {
                    AgentPhase::ConfigureRequirementsApproval
                }
                AgentPhase::ChooseBasePackApproval | AgentPhase::BasePackSearch => {
                    AgentPhase::ChooseBasePackApproval
                }
                _ => AgentPhase::ConfirmCustomizationApproval,
            },
        }
    }
}

const INVALIDATION_RULES: &[InvalidationRule] = &[
    InvalidationRule {
        changed: ChangedField::MinecraftVersion,
        invalidates: TARGET_INVALIDATES,
        target: InvalidationTarget::ConfigureRequirementsApproval,
    },
    InvalidationRule {
        changed: ChangedField::Loader,
        invalidates: TARGET_INVALIDATES,
        target: InvalidationTarget::ConfigureRequirementsApproval,
    },
    InvalidationRule {
        changed: ChangedField::VersionRequirement,
        invalidates: TARGET_INVALIDATES,
        target: InvalidationTarget::ConfigureRequirementsApproval,
    },
    InvalidationRule {
        changed: ChangedField::ContentPreference,
        invalidates: CONTENT_INVALIDATES,
        target: InvalidationTarget::ContentPreference,
    },
    InvalidationRule {
        changed: ChangedField::SearchPreference,
        invalidates: TARGET_INVALIDATES,
        target: InvalidationTarget::ChooseBasePackApproval,
    },
    InvalidationRule {
        changed: ChangedField::BasePack,
        invalidates: CONTENT_INVALIDATES,
        target: InvalidationTarget::ChooseBasePackApproval,
    },
];

pub(super) fn invalidation_rule_for_changed_field(
    changed: ChangedField,
) -> &'static InvalidationRule {
    debug_assert_eq!(INVALIDATION_RULES.len(), ALL_CHANGED_FIELDS.len());
    INVALIDATION_RULES
        .iter()
        .find(|rule| rule.changed == changed)
        .expect("every ChangedField must have an invalidation rule")
}

fn invalidates_for_changed_field(changed: ChangedField) -> Vec<PlanArtifact> {
    invalidation_rule_for_changed_field(changed)
        .invalidates
        .to_vec()
}

pub(super) fn target_phase_for_changed_field(
    changed: ChangedField,
    from_phase: &AgentPhase,
) -> AgentPhase {
    invalidation_rule_for_changed_field(changed)
        .target
        .phase(from_phase)
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
    if clears_mod_plan(changed) {
        run.mod_plan = None;
    }

    let duplicate = run.replans.iter().any(|existing| {
        existing.from_phase == from_phase
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

fn clears_mod_plan(changed: ChangedField) -> bool {
    matches!(
        changed,
        ChangedField::MinecraftVersion
            | ChangedField::Loader
            | ChangedField::VersionRequirement
            | ChangedField::SearchPreference
            | ChangedField::BasePack
    )
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
    run.status = AgentStatus::Running;
    run.phase = AgentPhase::BasePackSearch;
    run.pending_approval = None;
    run.plan = None;
    run.push_message(
        AgentMessageKind::Assistant,
        format!("Requirements updated; searching again: {reason}"),
    );
    run.push_trace(
        "plan replan requested; invalidated downstream artifacts; continuing to base-pack search",
    );
    run
}

pub(super) async fn continue_after_requirements_confirmation(
    llm: &AgentLlmClient,
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
        format!(
            "Confirmed modpack requirements: {}",
            requirement_label(&restrictions)
        ),
    );
    run.restrictions = Some(restrictions);
    run.approved_build = None;
    run.execution = None;
    run.pending_approval = None;
    run.push_trace("approved normalized build requirements");
    continue_to_base_pack_search(llm, run).await
}

pub(super) async fn continue_after_requirements_feedback(
    llm: &AgentLlmClient,
    mut run: AgentRunSnapshot,
    feedback: &str,
) -> Result<AgentRunSnapshot> {
    run.push_message(
        AgentMessageKind::User,
        format!("Changed modpack requirements: {feedback}"),
    );
    run.pending_approval = None;
    run.push_trace("received build requirement feedback; updating restrictions");

    let current = run.restrictions.clone().unwrap_or_default();
    let generated = generate_restriction_update(
        llm,
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
    run.push_trace(format!(
        "llm generated build restriction update via {}",
        generated.model
    ));
    run.push_message(
        AgentMessageKind::Assistant,
        requirement_summary_message(&output),
    );
    let run = apply_requirements_replan(
        run,
        output,
        format!("requirements revised: {feedback}"),
        AgentPhase::ConfigureRequirementsApproval,
    );
    continue_to_base_pack_search(llm, run).await
}

pub(super) async fn maybe_replan_requirements_from_feedback(
    llm: &AgentLlmClient,
    mut run: AgentRunSnapshot,
    feedback: &str,
) -> Result<Option<AgentRunSnapshot>> {
    let current = run.restrictions.clone().unwrap_or_default();
    let generated = generate_restriction_update(
        llm,
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
    run.push_message(
        AgentMessageKind::User,
        format!("Changed customization requirements: {feedback}"),
    );
    let mut replanned = apply_requirements_replan(
        run,
        output,
        format!("user changed version/loader during customization: {feedback}"),
        from_phase,
    );
    replanned.push_trace(format!(
        "llm generated build restriction replan via {}",
        generated.model
    ));
    Ok(Some(continue_to_base_pack_search(llm, replanned).await?))
}
