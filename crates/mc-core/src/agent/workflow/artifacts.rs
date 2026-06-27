use super::*;

pub(super) fn candidate_option(candidate: &BasePackCandidate) -> ApprovalOption {
    let hit = &candidate.hit;
    let provider = provider_slug(candidate.provider);
    let describe = project_describe(candidate.provider, hit, &candidate.matched_query);
    let mut payload = serde_json::json!({
        "provider": provider,
        "project_id": hit.id,
        "slug": hit.slug,
        "title": hit.title,
        "description": hit.description,
        "describe": describe,
        "author": hit.author,
        "downloads": hit.downloads,
        "icon_url": hit.icon_url,
        "gallery_url": hit.gallery_url,
        "categories": hit.categories,
        "url": project_url(candidate.provider, ResourceKind::Modpack, &hit.slug),
        "matched_query": candidate.matched_query,
    });
    if let (Some(obj), Some(target)) = (payload.as_object_mut(), candidate.resolved_target.as_ref())
    {
        obj.insert(
            "resolved_version".to_string(),
            target_resolved_version_payload(target),
        );
        if let Some(size) = target.primary_file.as_ref().and_then(|file| file.size) {
            obj.insert("archive_size".to_string(), serde_json::json!(size));
        }
    }
    ApprovalOption {
        id: format!("{provider}:{}", hit.id),
        label: hit.title.clone(),
        description: Some(describe.clone()),
        payload: Some(payload),
    }
}

pub(super) fn attach_base_pack_resolution(
    base_pack_payload: &mut serde_json::Value,
    base: &SelectedBasePack,
    target: &TargetCompatibility,
) {
    let Some(obj) = base_pack_payload.as_object_mut() else {
        return;
    };
    let resolved_version = target_resolved_version_payload(target);
    obj.insert("resolved_version".to_string(), resolved_version.clone());
    obj.insert(
        "source_ref".to_string(),
        serde_json::json!({
            "kind": "base_modpack_archive",
            "provider": provider_slug(base.provider),
            "project_id": base.project_id.clone(),
            "slug": base.slug.clone(),
            "title": base.title.clone(),
            "resolved_version": resolved_version,
            "archive_file": target.primary_file.as_ref().map(version_file_payload),
            "modlist_strategy": "download_base_archive_then_parse_modlist",
        }),
    );
}

fn target_resolved_version_payload(target: &TargetCompatibility) -> serde_json::Value {
    serde_json::json!({
        "version_id": target.version_id.clone(),
        "version_name": target.version_name.clone(),
        "version_number": target.version_number.clone(),
        "game_versions": target.game_versions.clone(),
        "loaders": target.loaders.clone(),
        "primary_file": target.primary_file.as_ref().map(version_file_payload),
        "dependencies": target.dependencies.clone(),
    })
}

pub(super) fn version_file_payload(file: &VersionFile) -> serde_json::Value {
    serde_json::json!({
        "url": file.url.clone(),
        "filename": file.filename.clone(),
        "sha1": file.sha1.clone(),
        "sha512": file.sha512.clone(),
        "size": file.size,
        "primary": file.primary,
        "client_side": file.client_side,
        "server_side": file.server_side,
    })
}

#[cfg(test)]
pub(super) fn resolved_mod_payload(resolved: &ResolvedModCandidate) -> serde_json::Value {
    let candidate = &resolved.candidate;
    let provider = provider_slug(candidate.provider);
    let hit = &candidate.hit;
    let describe = project_describe(candidate.provider, hit, &candidate.matched_query);
    let file = version_file_with_project_side(&resolved.file, hit);
    let file_payload = version_file_payload(&file);
    let review_reason = format!("matched {}", candidate.matched_query);
    serde_json::json!({
        "provider": provider,
        "project_id": hit.id.clone(),
        "slug": hit.slug.clone(),
        "title": hit.title.clone(),
        "description": hit.description.clone(),
        "describe": describe,
        "author": hit.author.clone(),
        "downloads": hit.downloads,
        "icon_url": hit.icon_url.clone(),
        "gallery_url": hit.gallery_url.clone(),
        "categories": hit.categories.clone(),
        "url": project_url(candidate.provider, ResourceKind::Mod, &hit.slug),
        "matched_query": candidate.matched_query.clone(),
        "auto_added": false,
        "dependency_reason": "root_candidate",
        "review_source": "selected_candidate",
        "review_reason": review_reason,
        "review_version": resolved.version.version_number.clone(),
        "review_file": file.filename.clone(),
        "resolved_version": {
            "version_id": resolved.version.id.clone(),
            "version_name": resolved.version.name.clone(),
            "version_number": resolved.version.version_number.clone(),
            "game_versions": resolved.version.game_versions.clone(),
            "loaders": resolved.version.loaders.clone(),
            "primary_file": file_payload,
            "dependencies": resolved.version.dependencies.clone(),
        },
        "source_ref": {
            "kind": "mod_file",
            "provider": provider,
            "project_id": hit.id.clone(),
            "version_id": resolved.version.id.clone(),
            "file": version_file_payload(&file),
        },
    })
}

#[cfg(test)]
pub(super) fn mod_payload(candidate: &ModCandidate) -> serde_json::Value {
    let provider = provider_slug(candidate.provider);
    let hit = &candidate.hit;
    let describe = project_describe(candidate.provider, hit, &candidate.matched_query);
    serde_json::json!({
        "provider": provider,
        "project_id": hit.id,
        "slug": hit.slug,
        "title": hit.title,
        "description": hit.description,
        "describe": describe,
        "author": hit.author,
        "downloads": hit.downloads,
        "icon_url": hit.icon_url,
        "gallery_url": hit.gallery_url,
        "categories": hit.categories,
        "url": project_url(candidate.provider, ResourceKind::Mod, &hit.slug),
        "matched_query": candidate.matched_query,
    })
}

pub(super) fn safe_provider_filename(raw: &str) -> Option<String> {
    let normalized = raw.trim().replace('\\', "/");
    let basename = normalized
        .rsplit('/')
        .find(|part| !part.trim().is_empty())?
        .trim();
    if basename == "." || basename == ".." {
        return None;
    }
    let sanitized = crate::fs::sanitize_filename(basename, '-');
    if sanitized.trim().is_empty()
        || sanitized == "."
        || sanitized == ".."
        || sanitized.contains('/')
        || sanitized.contains('\\')
    {
        return None;
    }
    Some(sanitized)
}

#[cfg(test)]
pub(super) fn mrpack_file_payload(file: &VersionFile) -> Option<serde_json::Value> {
    let safe_filename = safe_provider_filename(&file.filename)?;
    mrpack_file_payload_with_filename(file, &safe_filename)
}

pub(super) fn mrpack_file_payload_with_filename(
    file: &VersionFile,
    safe_filename: &str,
) -> Option<serde_json::Value> {
    let sha512 = file.sha512.as_deref().filter(|s| !s.trim().is_empty())?;
    if file.url.trim().is_empty() || !host_in_whitelist(&file.url) {
        return None;
    }
    Some(serde_json::json!({
        "path": format!("mods/{safe_filename}"),
        "downloads": [file.url.clone()],
        "hashes": {
            "sha512": sha512,
            "sha1": file.sha1.clone(),
        },
        "fileSize": file.size,
        "env": {
            "client": file.client_side.as_mrpack_env(),
            "server": file.server_side.as_mrpack_env(),
        }
    }))
}

pub(super) fn version_file_with_project_side(file: &VersionFile, hit: &SearchHit) -> VersionFile {
    let mut file = file.clone();
    file.client_side = hit.client_side;
    file.server_side = hit.server_side;
    file
}

fn project_describe(provider: ProviderId, hit: &SearchHit, matched_query: &str) -> String {
    format!(
        "{} | by {} | {} downloads | matched: {} | {}",
        provider_label(provider),
        hit.author,
        hit.downloads,
        matched_query,
        hit.description
    )
}

pub(super) fn source_ref_payload(value: &serde_json::Value) -> Option<serde_json::Value> {
    value
        .get("source_ref")
        .or_else(|| value.get("execution_source"))
        .cloned()
}

fn execution_recipe_payload(
    base_pack: &serde_json::Value,
    target: &TargetCompatibility,
    extra_mods: &[serde_json::Value],
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "kind": "mrpack_from_base_modpack",
        "format": "mrpack",
        "compile_phase": "exec.compile_execution_manifest",
        "target": {
            "minecraft_version": target.minecraft_version.clone(),
            "loader": target.loader.clone(),
            "base_version_id": target.version_id.clone(),
            "base_version_name": target.version_name.clone(),
            "base_version_number": target.version_number.clone(),
        },
        "base_pack_ref": {
            "provider": optional_json_string(base_pack, "provider"),
            "project_id": optional_json_string(base_pack, "project_id"),
            "slug": optional_json_string(base_pack, "slug"),
            "title": optional_json_string(base_pack, "title"),
            "resolved_version": base_pack.get("resolved_version").cloned(),
            "source_ref": source_ref_payload(base_pack),
        },
        "extra_mod_refs": extra_mods
            .iter()
            .map(|m| serde_json::json!({
                "provider": optional_json_string(m, "provider"),
                "project_id": optional_json_string(m, "project_id"),
                "slug": optional_json_string(m, "slug"),
                "title": optional_json_string(m, "title"),
                "resolved_version": m.get("resolved_version").cloned(),
                "source_ref": source_ref_payload(m),
            }))
            .collect::<Vec<_>>(),
        "compile_contract": {
            "input": "download base_pack_ref.source_ref.archive_file, parse its modrinth.index.json and preserve base archive overrides",
            "merge": "append compatible extra_mod_refs to the parsed base modlist; remote-eligible files go to modrinth.index.json, non-whitelisted downloadable files go to overrides",
            "dedupe": "dedupe exact output paths first; executor may add provider/project_id dedupe after parsing richer base metadata",
            "output": "execution.metadata.manifest, not the approved plan payload"
        }
    })
}

pub(super) fn selection_plan(
    user_prompt: &str,
    queries: &[String],
    candidates: &[BasePackCandidate],
) -> ModpackAgentPlan {
    let top = candidates
        .iter()
        .take(3)
        .map(|c| format!("- {} ({})", c.hit.title, c.hit.slug))
        .collect::<Vec<_>>()
        .join("\n");
    let summary_markdown = if candidates.is_empty() {
        format!(
            "No base-pack candidates were found.\n\nQueries:\n- {}\n\nChange the version, loader, or requirement tags and search again.",
            queries.join("\n- ")
        )
    } else {
        format!(
            "User request: {user_prompt}\n\nExisting modpacks were searched first as base candidates.\n\nQueries:\n- {}\n\nCandidate preview:\n{}",
            queries.join("\n- "),
            top
        )
    };

    ModpackAgentPlan {
        objective: user_prompt.to_string(),
        summary_markdown,
        risks: vec![
            "Candidates still need Minecraft version, loader, and extra-mod compatibility checks in the next step.".to_string(),
            "Base-pack selection is a HITL gate; import/install/write does not run before confirmation.".to_string(),
        ],
        planned_actions: vec![
            PlannedAction {
                id: "choose-base-pack".to_string(),
                label: "User chooses one base modpack".to_string(),
                tool: "approval_gate".to_string(),
                args: serde_json::json!({ "kind": "choose_base_pack" }),
                requires_approval: true,
            },
            PlannedAction {
                id: "plan-extra-mods".to_string(),
                label: "Search compatible customization mods".to_string(),
                tool: "search_mods".to_string(),
                args: serde_json::json!({ "after": "base_pack_selected" }),
                requires_approval: false,
            },
        ],
        migration_notes: vec![
            "Provider search stays in daemon/core; future sidecar should call this as a tool."
                .to_string(),
            "ApprovalRequest/UserDecision remain the UI-independent HITL contract.".to_string(),
        ],
    }
}

pub(super) fn customization_approval(
    user_prompt: &str,
    base: &SelectedBasePack,
    target: &TargetCompatibility,
    base_pack: serde_json::Value,
    extra_mods: Vec<serde_json::Value>,
) -> (ModpackAgentPlan, ApprovalRequest) {
    customization_approval_with_validation(user_prompt, base, target, base_pack, extra_mods, None)
}

pub(super) fn customization_approval_with_validation(
    user_prompt: &str,
    base: &SelectedBasePack,
    target: &TargetCompatibility,
    base_pack: serde_json::Value,
    extra_mods: Vec<serde_json::Value>,
    validation: Option<serde_json::Value>,
) -> (ModpackAgentPlan, ApprovalRequest) {
    let plan = customization_plan(user_prompt, base, target, &extra_mods);
    let execution_recipe = execution_recipe_payload(&base_pack, target, &extra_mods);
    let approval = ApprovalRequest {
        id: crate::agent::state::new_id("approval"),
        kind: ApprovalKind::ConfirmCustomization,
        title: "Confirm customization plan".to_string(),
        message: "After confirmation, deterministic execution can write the artifact. This step only prepares the plan.".to_string(),
        options: vec![
            ApprovalOption {
                id: "confirm:recommended_customization".to_string(),
                label: "Confirm recommended plan".to_string(),
                description: Some(format!(
                    "Base pack: {}; extra mods: {}",
                    base.title,
                    extra_mods.len()
                )),
                payload: Some(serde_json::json!({
                    "base_pack": base_pack,
                    "target": {
                        "minecraft_version": target.minecraft_version.clone(),
                        "loader": target.loader.clone(),
                        "base_version_id": target.version_id.clone(),
                        "base_version_name": target.version_name.clone(),
                        "base_version_number": target.version_number.clone(),
                        "base_game_versions": target.game_versions.clone(),
                        "base_loaders": target.loaders.clone(),
                        "base_primary_file": target.primary_file.as_ref().map(version_file_payload),
                    },
                    "extra_mods": extra_mods,
                    "validation": validation,
                    "execution_recipe": execution_recipe,
                })),
            },
            ApprovalOption {
                id: "back:choose_base_pack".to_string(),
                label: "Back to base-pack selection".to_string(),
                description: Some(
                    "The current candidate is not suitable; return to base-pack selection."
                        .to_string(),
                ),
                payload: Some(serde_json::json!({ "action": "back_to_base_pack" })),
            },
        ],
        available_decisions: approval_decisions("Confirm recommended plan", "Change extra mods"),
        tools: Vec::new(),
        plan: Some(plan.clone()),
    };
    (plan, approval)
}

fn customization_plan(
    user_prompt: &str,
    base: &SelectedBasePack,
    target: &TargetCompatibility,
    mods: &[serde_json::Value],
) -> ModpackAgentPlan {
    let target_text = match (&target.minecraft_version, &target.loader) {
        (Some(mc), Some(loader)) => format!("MC {mc} / {loader}"),
        (Some(mc), None) => format!("MC {mc}"),
        (None, Some(loader)) => loader.clone(),
        (None, None) => "unknown compatibility target".to_string(),
    };
    let mods_text = if mods.is_empty() {
        "- No compatible extra mod candidates yet".to_string()
    } else {
        mods.iter()
            .take(6)
            .map(|m| {
                format!(
                    "- {} ({}, {} downloads)",
                    json_str_or(m, "title", "unknown"),
                    json_str_or(m, "slug", "unknown"),
                    m.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    ModpackAgentPlan {
        objective: user_prompt.to_string(),
        summary_markdown: format!(
            "Selected base pack: {} ({})\n\nCompatibility target: {}\n\nRecommended extra mods:\n{}",
            base.title, base.slug, target_text, mods_text
        ),
        risks: vec![
            "Extra mods come from provider search results and still need final version-file resolution before execution.".to_string(),
            "Import/install/write does not run before the customization plan is confirmed.".to_string(),
        ],
        planned_actions: vec![
            PlannedAction {
                id: "confirm-customization".to_string(),
                label: "User confirms base pack plus extra mods".to_string(),
                tool: "approval_gate".to_string(),
                args: serde_json::json!({ "kind": "confirm_customization" }),
                requires_approval: true,
            },
            PlannedAction {
                id: "execute-install".to_string(),
                label: "Import base modpack and install approved extra mods".to_string(),
                tool: "install_modpack_with_mod_overrides".to_string(),
                args: serde_json::json!({ "after": "customization_confirmed" }),
                requires_approval: false,
            },
        ],
        migration_notes: vec![
            "Session remains daemon-owned; continue reads and writes the same snapshot file."
                .to_string(),
        ],
    }
}

pub(super) fn json_str_or<'a>(
    value: &'a serde_json::Value,
    field: &str,
    fallback: &'a str,
) -> &'a str {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback)
}

pub(super) fn scratch_fallback_unavailable_plan(user_prompt: &str) -> ModpackAgentPlan {
    ModpackAgentPlan {
        objective: user_prompt.to_string(),
        summary_markdown:
            "The scratch-build branch is not implemented yet. Change the base-pack search requirements and search again from the base-pack gate.".to_string(),
        risks: vec!["The current workflow can continue planning only from an existing base pack."
            .to_string()],
        planned_actions: vec![PlannedAction {
            id: "revise-base-pack-search".to_string(),
            label: "User revises base pack search requirements".to_string(),
            tool: "approval_gate".to_string(),
            args: serde_json::json!({ "kind": "choose_base_pack", "scratch_fallback_unavailable": true }),
            requires_approval: true,
        }],
        migration_notes: vec![],
    }
}

pub(super) fn provider_slug(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Modrinth => "modrinth",
        ProviderId::CurseForge => "curseforge",
    }
}

pub(super) fn provider_label(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Modrinth => "Modrinth",
        ProviderId::CurseForge => "CurseForge",
    }
}

pub(super) fn project_url(provider: ProviderId, kind: ResourceKind, slug: &str) -> String {
    match provider {
        ProviderId::Modrinth => match kind {
            ResourceKind::Modpack => format!("https://modrinth.com/modpack/{slug}"),
            _ => format!("https://modrinth.com/mod/{slug}"),
        },
        ProviderId::CurseForge => match kind {
            ResourceKind::Modpack => {
                format!("https://www.curseforge.com/minecraft/modpacks/{slug}")
            }
            _ => format!("https://www.curseforge.com/minecraft/mc-mods/{slug}"),
        },
    }
}
