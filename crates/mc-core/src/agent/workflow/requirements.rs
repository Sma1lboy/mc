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
    // The genuine schema/parse failure happens *inside* `prompt_typed`
    // (`serde_json::from_str` of the raw model text), so the retry must hinge on
    // this `Err`, not on the post-parse validators (which are infallible).
    let first_attempt = llm
        .prompt_typed::<UpdateBuildRestrictionsInput>(
            &[MAIN_AGENT_SYSTEM_PROMPT, REQUIREMENT_NORMALIZATION_PROMPT],
            input,
            260,
            0.0,
        )
        .await;
    let input = match first_attempt {
        Ok(response) => validate_restriction_update_input(response)?,
        Err(first_err) => {
            // Re-prompt once, feeding back the genuine error string the model
            // failed on (the raw parse/schema violation, not a Debug dump of an
            // already-parsed struct) so it can correct the malformed output.
            let schema_violation = first_err.to_string();
            let retry_input = restriction_update_request_payload(
                original_user_prompt,
                current,
                user_message,
                source,
                Some(schema_violation.clone()),
                Some(schema_violation),
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
                .await
                .map_err(|second_err| {
                    CoreError::other(format!(
                        "could not parse restriction tool schema output after retry: {second_err}; first error: {first_err}"
                    ))
                })?;
            validate_restriction_update_retry(retry, &first_err)?
        }
    };
    let model = llm.model().to_string();
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
    run.enter_phase(AgentPhase::BasePackSearch);
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

#[cfg(test)]
mod retry_tests {
    use super::*;

    /// Minimal local HTTP server that serves two sequential responses. The shared
    /// `one_response_server` test helpers only answer a single connection; the
    /// schema-retry path issues two `prompt_typed` calls, so we need to answer the
    /// initial attempt and the retry. Each response sets `Connection: close`, so
    /// the client opens a fresh connection per call and the bodies are consumed in
    /// order.
    fn two_response_server(first: Vec<u8>, second: Vec<u8>) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for body in [first, second] {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buf = [0_u8; 16384];
                let _ = stream.read(&mut buf);
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        format!("http://{addr}")
    }

    /// Wrap raw assistant text in an OpenRouter chat-completion envelope. `content`
    /// is returned verbatim as the model output, so passing non-JSON text exercises
    /// the genuine `prompt_typed` parse failure (its internal `serde_json::from_str`
    /// rejects it).
    fn openrouter_response_body(content: &str) -> Vec<u8> {
        serde_json::json!({
            "id": "chatcmpl_test",
            "object": "chat.completion",
            "created": 0,
            "model": "gpt-test",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": content },
                "finish_reason": "stop",
                "native_finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })
        .to_string()
        .into_bytes()
    }

    fn client_for(base_url: String) -> AgentLlmClient {
        let mut cfg = crate::agent::AgentLlmConfig::new("test-key");
        cfg.base_url = base_url;
        crate::agent::AgentLlmClient::new(cfg).unwrap()
    }

    /// Well-formed `UpdateBuildRestrictionsInput` JSON (pre-normalization): a raw
    /// loader case, padded/duplicate feature tags, and padded notes so the test can
    /// assert that post-parse normalization still runs on the retry value.
    const VALID_RETRY_OUTPUT: &str = r#"{"base_revision":0,"patch":{"minecraft_version":"1.20.1","loader":"Fabric","feature_tags":[" performance ","performance"],"notes":"  keep this  "}}"#;

    #[tokio::test]
    async fn schema_retry_recovers_after_one_malformed_response() {
        // First attempt returns text that is not valid JSON for the schema, so the
        // initial `prompt_typed` errors inside its `serde_json::from_str`. The retry
        // returns a valid object; the function must return it after exactly one retry.
        let base_url = two_response_server(
            openrouter_response_body("this is not a json object"),
            openrouter_response_body(VALID_RETRY_OUTPUT),
        );
        let llm = client_for(base_url);

        let generated = generate_restriction_update(
            &llm,
            "make me a fabric pack",
            &BuildRestrictions::default(),
            "make me a fabric pack",
            BuildRestrictionChangeSource::InitialPrompt,
        )
        .await
        .expect("retry should recover from a malformed first response");

        // Post-parse normalization is applied to the successful retry value.
        assert_eq!(generated.input.base_revision, 0);
        assert_eq!(
            generated.input.patch.minecraft_version.as_deref(),
            Some("1.20.1")
        );
        assert_eq!(
            generated
                .input
                .patch
                .minecraft_version_requirement
                .as_deref(),
            Some("1.20.1")
        );
        assert_eq!(generated.input.patch.loader.as_deref(), Some("fabric"));
        assert_eq!(generated.input.patch.feature_tags, vec!["performance"]);
        assert_eq!(generated.input.patch.notes.as_deref(), Some("keep this"));
    }

    #[tokio::test]
    async fn schema_retry_surfaces_clear_error_when_retry_also_malformed() {
        // Both attempts are malformed: the initial parse fails, the retry parse also
        // fails, and the function surfaces a clear error after the single retry.
        let base_url = two_response_server(
            openrouter_response_body("still not json"),
            openrouter_response_body("also not json"),
        );
        let llm = client_for(base_url);

        let err = generate_restriction_update(
            &llm,
            "make me a pack",
            &BuildRestrictions::default(),
            "make me a pack",
            BuildRestrictionChangeSource::InitialPrompt,
        )
        .await
        .expect_err("a persistently malformed response must surface an error after the retry");

        let message = err.to_string();
        assert!(
            message.contains("after retry"),
            "error should report that the retry also failed: {message}"
        );
    }
}
