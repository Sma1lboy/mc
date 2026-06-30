use super::*;

pub(super) fn requirements_approval(
    user_prompt: &str,
    output: &UpdateBuildRestrictionsOutput,
) -> ApprovalRequest {
    let restrictions = &output.restrictions;
    let missing = &output.missing_fields;
    ApprovalRequest {
        id: crate::agent::state::new_id("approval"),
        kind: ApprovalKind::ConfigureRequirements,
        title: "Confirm modpack requirements".to_string(),
        message: if missing.is_empty() {
            "Review the Minecraft version, loader, and requirement tags. Base-pack search starts after confirmation.".to_string()
        } else {
            format!(
                "Review the normalized requirement tags. Missing: {}. You can add details, or continue with broader search constraints.",
                missing_fields_label(missing)
            )
        },
        options: vec![ApprovalOption {
            id: "requirements:detected".to_string(),
            label: requirement_label(restrictions),
            description: Some(requirement_description(output)),
            payload: Some(requirement_payload(output)),
        }],
        available_decisions: requirement_decisions(),
        tools: vec![update_build_restrictions_tool_spec()],
        plan: Some(requirements_plan(user_prompt, output)),
    }
}

fn requirement_decisions() -> Vec<ApprovalDecisionSpec> {
    vec![
        ApprovalDecisionSpec {
            kind: UserDecisionKind::Approve,
            label: "Confirm and continue".to_string(),
            requires_selected_option: true,
            requires_message: false,
        },
        ApprovalDecisionSpec {
            kind: UserDecisionKind::Revise,
            label: "Add or change requirements".to_string(),
            requires_selected_option: false,
            requires_message: true,
        },
        ApprovalDecisionSpec {
            kind: UserDecisionKind::Cancel,
            label: "Cancel".to_string(),
            requires_selected_option: false,
            requires_message: false,
        },
    ]
}

fn requirement_payload(output: &UpdateBuildRestrictionsOutput) -> serde_json::Value {
    let restrictions = &output.restrictions;
    serde_json::json!({
        "tool": UPDATE_BUILD_RESTRICTIONS_TOOL,
        "minecraft_version": restrictions.minecraft_version.clone(),
        "minecraft_version_requirement": restrictions.minecraft_version_requirement.clone(),
        "loader": restrictions.loader.clone(),
        "feature_tags": restrictions.feature_tags.clone(),
        "missing_fields": output.missing_fields.clone(),
        "warnings": output.warnings.clone(),
        "notes": restrictions.notes.clone(),
        "revision": restrictions.revision,
        "restrictions": restrictions,
    })
}

pub(super) fn restrictions_from_requirement_payload(
    value: &serde_json::Value,
) -> Option<BuildRestrictions> {
    if let Some(restrictions) = value.get("restrictions") {
        if let Ok(parsed) = serde_json::from_value::<BuildRestrictions>(restrictions.clone()) {
            return Some(parsed);
        }
    }

    Some(BuildRestrictions {
        revision: value.get("revision").and_then(|v| v.as_u64()).unwrap_or(0),
        minecraft_version: optional_json_string(value, "minecraft_version")
            .filter(|v| is_minecraft_version(v)),
        minecraft_version_requirement: optional_json_string(value, "minecraft_version_requirement"),
        loader: value
            .get("loader")
            .and_then(|v| v.as_str())
            .and_then(normalize_loader),
        feature_tags: value
            .get("feature_tags")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        notes: optional_json_string(value, "notes"),
        history: Vec::new(),
    })
}

pub(super) fn missing_restriction_fields(restrictions: &BuildRestrictions) -> Vec<String> {
    let mut missing = Vec::new();
    if restrictions.minecraft_version.is_none() {
        missing.push("minecraft_version".to_string());
    }
    if restrictions.loader.is_none() {
        missing.push("loader".to_string());
    }
    missing
}

pub(super) fn requirement_label(restrictions: &BuildRestrictions) -> String {
    let mc = restrictions
        .minecraft_version
        .as_deref()
        .unwrap_or("Minecraft version not selected");
    let loader = restrictions
        .loader
        .as_deref()
        .unwrap_or("loader not selected");
    format!("{loader} / {mc}")
}

fn missing_fields_label(fields: &[String]) -> String {
    fields
        .iter()
        .map(|field| match field.as_str() {
            "minecraft_version" => "Minecraft version",
            "loader" => "loader",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn requirement_description(output: &UpdateBuildRestrictionsOutput) -> String {
    let restrictions = &output.restrictions;
    let tags = if restrictions.feature_tags.is_empty() {
        "no extra tags".to_string()
    } else {
        restrictions.feature_tags.join(", ")
    };
    let missing = &output.missing_fields;
    let warnings = if output.warnings.is_empty() {
        String::new()
    } else {
        format!("; warning: {}", output.warnings.join(", "))
    };
    if missing.is_empty() {
        format!("Requirement tags: {tags}{warnings}")
    } else {
        format!(
            "Requirement tags: {tags}; missing: {}{warnings}",
            missing_fields_label(missing)
        )
    }
}

pub(super) fn requirement_summary_message(output: &UpdateBuildRestrictionsOutput) -> String {
    format!(
        "Prepared modpack requirements: {}. {}",
        requirement_label(&output.restrictions),
        requirement_description(output)
    )
}

pub(super) fn requirements_plan(
    user_prompt: &str,
    output: &UpdateBuildRestrictionsOutput,
) -> ModpackAgentPlan {
    let restrictions = &output.restrictions;
    let missing = &output.missing_fields;
    ModpackAgentPlan {
        objective: user_prompt.to_string(),
        summary_markdown: format!(
            "Requirements confirmation:\n- {}\n- {}\n\nBase-pack search starts only after confirmation.",
            requirement_label(restrictions),
            requirement_description(output)
        ),
        risks: if missing.is_empty() {
            vec!["No search/export/write work runs before requirements are confirmed."
                .to_string()]
        } else {
            vec![format!(
                "Missing requirements: {}. If confirmed, search continues with broader constraints and relaxed compatibility filters.",
                missing_fields_label(missing)
            )]
        },
        planned_actions: vec![
            PlannedAction {
                id: "update-build-restrictions".to_string(),
                label: "Validate and store typed build restrictions".to_string(),
                tool: UPDATE_BUILD_RESTRICTIONS_TOOL.to_string(),
                args: serde_json::json!({ "revision": restrictions.revision }),
                requires_approval: false,
            },
            PlannedAction {
                id: "confirm-requirements".to_string(),
                label: "User confirms Minecraft version, loader, and requirement tags".to_string(),
                tool: "approval_gate".to_string(),
                args: serde_json::json!({ "kind": "configure_requirements" }),
                requires_approval: true,
            },
            PlannedAction {
                id: "search-base-pack".to_string(),
                label: "Search base modpacks after requirements are confirmed".to_string(),
                tool: "search_modpacks".to_string(),
                args: serde_json::json!({ "after": "requirements_confirmed" }),
                requires_approval: false,
            },
        ],
        migration_notes: vec![
            "Build restrictions are stored as typed workflow state before provider tools run."
                .to_string(),
            "UI should render missing_fields as review warnings for this audit-only CLI gate."
                .to_string(),
        ],
    }
}

pub(super) fn base_pack_selection_approval(
    candidates: &[BasePackCandidate],
    plan: ModpackAgentPlan,
) -> ApprovalRequest {
    ApprovalRequest {
        id: crate::agent::state::new_id("approval"),
        kind: ApprovalKind::ChooseBasePack,
        title: "Choose a base modpack or start from scratch".to_string(),
        message: if candidates.is_empty() {
            "The current search returned no existing base-pack candidates. Start from scratch, or change the version, loader, or requirement tags and search again.".to_string()
        } else {
            "Choose an existing modpack as the base, or start from scratch with an empty mod set. The next step plans compatible mods for the confirmed target.".to_string()
        },
        options: approval_options(candidates),
        available_decisions: approval_decisions("Choose this option", "Search base packs again"),
        tools: vec![update_build_restrictions_tool_spec()],
        plan: Some(plan),
    }
}

pub(super) fn approval_decisions(
    approve_label: &str,
    revise_label: &str,
) -> Vec<ApprovalDecisionSpec> {
    vec![
        ApprovalDecisionSpec {
            kind: UserDecisionKind::Approve,
            label: approve_label.to_string(),
            requires_selected_option: true,
            requires_message: false,
        },
        ApprovalDecisionSpec {
            kind: UserDecisionKind::Revise,
            label: revise_label.to_string(),
            requires_selected_option: false,
            requires_message: true,
        },
        ApprovalDecisionSpec {
            kind: UserDecisionKind::Cancel,
            label: "Cancel".to_string(),
            requires_selected_option: false,
            requires_message: false,
        },
    ]
}

fn approval_options(candidates: &[BasePackCandidate]) -> Vec<ApprovalOption> {
    let mut options = candidates.iter().map(candidate_option).collect::<Vec<_>>();
    options.push(scratch_base_pack_option());
    options
}

pub(super) fn approved_build_from_payload(
    payload: &serde_json::Value,
) -> Result<ApprovedModpackBuild> {
    let base_pack = payload
        .get("base_pack")
        .cloned()
        .ok_or_else(|| CoreError::other("approved plan missing base_pack"))?;
    let target = payload
        .get("target")
        .cloned()
        .ok_or_else(|| CoreError::other("approved plan missing target"))?;
    let extra_mods = payload
        .get("extra_mods")
        .and_then(|v| v.as_array())
        .map(|items| items.to_vec())
        .unwrap_or_default();
    let execution_recipe = payload
        .get("execution_recipe")
        .or_else(|| payload.get("mrpack_plan"))
        .cloned();
    Ok(ApprovedModpackBuild {
        base_pack,
        target,
        extra_mods,
        execution_recipe,
    })
}
