use super::*;

#[cfg(test)]
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
    let first_output = llm
        .prompt_text(
            &[
                MAIN_AGENT_SYSTEM_PROMPT,
                modpack_build_react_prompt(),
                REQUIREMENT_NORMALIZATION_PROMPT,
            ],
            input,
            260,
            0.0,
        )
        .await?;
    let first_attempt = parse_restriction_update_response(&first_output)
        .and_then(validate_restriction_update_input);
    let input = match first_attempt {
        Ok(response) => response,
        Err(first_err) => {
            let parse_error = first_err.to_string();
            let retry_input = restriction_update_request_payload(
                original_user_prompt,
                current,
                user_message,
                source,
                Some(parse_error),
                Some(first_output),
            )
            .to_string();
            let retry_output = llm
                .prompt_text(
                    &[
                        MAIN_AGENT_SYSTEM_PROMPT,
                        modpack_build_react_prompt(),
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
                        "could not request restriction tool arguments after retry: {second_err}; first error: {first_err}"
                    ))
                })?;
            let retry = parse_restriction_update_response(&retry_output).map_err(|second_err| {
                CoreError::other(format!(
                    "could not parse restriction tool arguments after retry: {second_err}; first error: {first_err}"
                ))
            })?;
            validate_restriction_update_retry(retry, &first_err)?
        }
    };
    Ok(GeneratedRestrictionUpdate { input })
}

#[cfg(test)]
pub(super) fn restriction_update_request_payload(
    original_user_prompt: &str,
    current: &BuildRestrictions,
    user_message: &str,
    source: BuildRestrictionChangeSource,
    parse_error: Option<String>,
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
        if let Some(parse_error) = parse_error {
            obj.insert("parse_error".to_string(), serde_json::json!(parse_error));
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
            "could not parse single restriction tool argument object: {err}: {text}"
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

#[cfg(test)]
fn validate_restriction_update_input(
    input: UpdateBuildRestrictionsInput,
) -> Result<UpdateBuildRestrictionsInput> {
    Ok(normalize_restriction_update_input(input))
}

#[cfg(test)]
pub(super) fn validate_restriction_update_retry(
    input: UpdateBuildRestrictionsInput,
    first_err: &CoreError,
) -> Result<UpdateBuildRestrictionsInput> {
    validate_restriction_update_input(input).map_err(|second_err| {
        CoreError::other(format!(
            "could not parse restriction tool arguments after retry: {second_err}; first error: {first_err}"
        ))
    })
}

#[cfg(test)]
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

/// Thin wrapper that routes a restriction update through the single authority,
/// [`BuildRestrictions::try_apply`]. Kept as a free fn so out-of-workflow callers
/// (e.g. base-pack feedback) can apply a patch against an `Option` snapshot
/// without owning a mutable `BuildRestrictions`.
pub(super) fn update_build_restrictions(
    current: Option<BuildRestrictions>,
    input: UpdateBuildRestrictionsInput,
    source: BuildRestrictionChangeSource,
    summary: impl Into<String>,
) -> Result<UpdateBuildRestrictionsOutput> {
    let mut restrictions = current.unwrap_or_default();
    restrictions.try_apply(input.base_revision, input.patch, source, summary)
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
mod retry_tests {
    use super::*;

    /// Minimal local HTTP server that serves two sequential responses. The shared
    /// `one_response_server` test helpers only answer a single connection; the
    /// parse-retry path issues two model calls, so we need to answer the initial
    /// attempt and the retry. Each response sets `Connection: close`, so the client
    /// opens a fresh connection per call and the bodies are consumed in order.
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
    /// the local tool-argument parser.
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
    async fn tool_argument_retry_recovers_after_one_malformed_response() {
        // First attempt returns text that is not valid JSON tool arguments. The retry
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
    async fn tool_argument_retry_surfaces_clear_error_when_retry_also_malformed() {
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
