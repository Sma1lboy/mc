use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::agent::build::parse_base_modlist;
use crate::agent::compatibility::{
    CompatibilityIssue, CompatibilityReport, IssueSeverity, SuggestedAction,
};
use crate::download::Downloader;
use crate::modplatform::{accepted_loaders, Dependency, ProjectVersion, ProviderId};

use super::{
    provider_from_slug, provider_slug, BuildBasePack, BuildModRef, BuildModpackArgs, BuildTarget,
    ChatToolError, ChatToolsCtx,
};

const MAX_BASE_ARCHIVE_BYTES: usize = 96 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ValidateModpackPlanArgs {
    pub target: BuildTarget,
    #[serde(default)]
    pub base_pack: Option<BuildBasePack>,
    #[serde(default)]
    pub extra_mods: Vec<BuildModRef>,
}

impl From<&BuildModpackArgs> for ValidateModpackPlanArgs {
    fn from(args: &BuildModpackArgs) -> Self {
        Self {
            target: args.target.clone(),
            base_pack: args.base_pack.clone(),
            extra_mods: args.extra_mods.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ValidateModpackPlanOutput {
    pub report: CompatibilityReport,
    pub checked_projects: usize,
}

struct SelectedVersion {
    provider: ProviderId,
    project_id: String,
    version: ProjectVersion,
}

pub async fn tool_validate_modpack_plan(
    ctx: &ChatToolsCtx,
    args: ValidateModpackPlanArgs,
) -> Result<ValidateModpackPlanOutput, ChatToolError> {
    let mut issues = Vec::new();
    let mut duplicate_projects: BTreeMap<String, usize> = BTreeMap::new();
    for mod_ref in &args.extra_mods {
        let provider = provider_from_slug(mod_ref.provider.as_deref().unwrap_or("modrinth"));
        *duplicate_projects
            .entry(project_key(provider, &mod_ref.project_id))
            .or_default() += 1;
    }
    for (key, count) in duplicate_projects
        .into_iter()
        .filter(|(_, count)| *count > 1)
    {
        issues.push(
            CompatibilityIssue::new(
                "duplicate_project",
                IssueSeverity::Blocking,
                format!("Project {key} appears {count} times in the build plan"),
            )
            .with_subjects([key.clone()])
            .with_suggested_actions([
                SuggestedAction::new("remove_duplicate_project").with_target(key)
            ]),
        );
    }

    let mut selected = Vec::new();
    for mod_ref in &args.extra_mods {
        let provider = provider_from_slug(mod_ref.provider.as_deref().unwrap_or("modrinth"));
        if let Some(version) =
            fetch_selected_version(ctx, provider, &mod_ref.project_id, &mod_ref.version_id).await?
        {
            append_version_issues(
                provider,
                &mod_ref.project_id,
                &version,
                &args.target,
                &mut issues,
            );
            selected.push(SelectedVersion {
                provider,
                project_id: mod_ref.project_id.clone(),
                version,
            });
        } else {
            issues.push(selected_version_issue(
                provider,
                &mod_ref.project_id,
                &mod_ref.version_id,
                "The selected version no longer exists",
            ));
        }
    }

    let mut base_projects = HashSet::new();
    if let Some(base_pack) = &args.base_pack {
        match fetch_selected_version(
            ctx,
            ProviderId::Modrinth,
            &base_pack.project_id,
            &base_pack.version_id,
        )
        .await?
        {
            Some(version) => {
                let version_is_compatible = append_version_issues(
                    ProviderId::Modrinth,
                    &base_pack.project_id,
                    &version,
                    &args.target,
                    &mut issues,
                );
                if version_is_compatible {
                    let file = version
                        .primary_file()
                        .expect("compatible versions always have a primary file");
                    let downloader = Downloader::new(2)?;
                    let archive = downloader
                        .get_bytes_capped(file.url.trim(), MAX_BASE_ARCHIVE_BYTES)
                        .await?;
                    match parse_base_modlist(&archive) {
                        Ok(refs) => {
                            base_projects.extend(
                                refs.into_iter()
                                    .map(|item| project_key(item.provider, &item.project_id)),
                            );
                        }
                        Err(error) => issues.push(selected_version_issue(
                            ProviderId::Modrinth,
                            &base_pack.project_id,
                            &base_pack.version_id,
                            format!("The base pack manifest could not be inspected: {error}"),
                        )),
                    }
                }
            }
            None => issues.push(selected_version_issue(
                ProviderId::Modrinth,
                &base_pack.project_id,
                &base_pack.version_id,
                "The selected base pack version no longer exists",
            )),
        }
    }

    let selected_versions = selected
        .iter()
        .map(|item| {
            (
                project_key(item.provider, &item.project_id),
                item.version.id.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    for key in selected_versions.keys() {
        if base_projects.contains(key) {
            issues.push(
                CompatibilityIssue::new(
                    "duplicate_project",
                    IssueSeverity::Blocking,
                    format!("Project {key} is already included in the base pack"),
                )
                .with_subjects([key.clone()])
                .with_suggested_actions([
                    SuggestedAction::new("remove_extra_mod").with_target(key.clone())
                ]),
            );
        }
    }
    append_dependency_issues(&selected, &selected_versions, &base_projects, &mut issues);

    Ok(ValidateModpackPlanOutput {
        report: CompatibilityReport::from_issues(issues),
        checked_projects: selected.len() + usize::from(args.base_pack.is_some()),
    })
}

async fn fetch_selected_version(
    ctx: &ChatToolsCtx,
    provider_id: ProviderId,
    project_id: &str,
    version_id: &str,
) -> Result<Option<ProjectVersion>, ChatToolError> {
    let provider = ctx.registry.get(provider_id).ok_or_else(|| {
        ChatToolError::new(format!(
            "provider {} is not registered",
            provider_slug(provider_id)
        ))
    })?;
    Ok(provider
        .list_versions(project_id, None, None)
        .await?
        .into_iter()
        .find(|version| version.id == version_id))
}

fn append_version_issues(
    provider: ProviderId,
    project_id: &str,
    version: &ProjectVersion,
    target: &BuildTarget,
    issues: &mut Vec<CompatibilityIssue>,
) -> bool {
    let mc_matches = version
        .game_versions
        .iter()
        .any(|game_version| game_version == target.mc_version.trim());
    let accepted = accepted_loaders(&target.loader);
    let loader_matches = version.loaders.iter().any(|loader| {
        accepted
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(loader))
    });
    let compatible = mc_matches && loader_matches && version.primary_file().is_some();
    if !compatible {
        let reason = format!(
            "Selected version {} targets mc={:?}, loaders={:?}; requested mc={}, loader={}",
            version.id, version.game_versions, version.loaders, target.mc_version, target.loader
        );
        issues.push(selected_version_issue(
            provider,
            project_id,
            &version.id,
            reason,
        ));
    }
    compatible
}

fn selected_version_issue(
    provider: ProviderId,
    project_id: &str,
    version_id: &str,
    summary: impl Into<String>,
) -> CompatibilityIssue {
    CompatibilityIssue::new(
        "selected_version_incompatible",
        IssueSeverity::Blocking,
        summary,
    )
    .with_subjects([format!(
        "{}:{project_id}:{version_id}",
        provider_slug(provider)
    )])
    .with_suggested_actions([SuggestedAction::new("resolve_compatible_version")
        .with_target(project_key(provider, project_id))])
}

fn append_dependency_issues(
    selected: &[SelectedVersion],
    selected_versions: &HashMap<String, String>,
    base_projects: &HashSet<String>,
    issues: &mut Vec<CompatibilityIssue>,
) {
    for item in selected {
        for dependency in &item.version.dependencies {
            match dependency.dependency_type.as_str() {
                "required"
                    if !dependency_is_satisfied(
                        item.provider,
                        dependency,
                        selected_versions,
                        base_projects,
                    ) =>
                {
                    issues.push(dependency_issue(
                        "missing_required_dependency",
                        item,
                        dependency,
                    ));
                }
                "incompatible"
                    if dependency_is_present(
                        item.provider,
                        dependency,
                        selected_versions,
                        base_projects,
                    ) =>
                {
                    issues.push(dependency_issue("declared_mod_conflict", item, dependency));
                }
                _ => {}
            }
        }
    }
}

fn dependency_is_satisfied(
    provider: ProviderId,
    dependency: &Dependency,
    selected_versions: &HashMap<String, String>,
    base_projects: &HashSet<String>,
) -> bool {
    if let Some(project_id) = dependency.project_id.as_deref() {
        let key = project_key(provider, project_id);
        if base_projects.contains(&key) {
            return true;
        }
        return selected_versions.get(&key).is_some_and(|selected_version| {
            dependency
                .version_id
                .as_deref()
                .is_none_or(|required_version| selected_version == required_version)
        });
    }
    dependency
        .version_id
        .as_deref()
        .is_some_and(|required_version| {
            selected_versions
                .values()
                .any(|selected_version| selected_version == required_version)
        })
}

fn dependency_is_present(
    provider: ProviderId,
    dependency: &Dependency,
    selected_versions: &HashMap<String, String>,
    base_projects: &HashSet<String>,
) -> bool {
    if let Some(project_id) = dependency.project_id.as_deref() {
        let key = project_key(provider, project_id);
        if base_projects.contains(&key) {
            return dependency.version_id.is_none();
        }
        return selected_versions.get(&key).is_some_and(|selected_version| {
            dependency
                .version_id
                .as_deref()
                .is_none_or(|conflicting_version| selected_version == conflicting_version)
        });
    }
    dependency
        .version_id
        .as_deref()
        .is_some_and(|conflicting_version| {
            selected_versions
                .values()
                .any(|selected_version| selected_version == conflicting_version)
        })
}

fn dependency_issue(
    code: &'static str,
    source: &SelectedVersion,
    dependency: &Dependency,
) -> CompatibilityIssue {
    let target = dependency
        .project_id
        .as_deref()
        .map(|project_id| project_key(source.provider, project_id))
        .or_else(|| dependency.version_id.clone())
        .unwrap_or_else(|| "unknown dependency".to_string());
    let summary = if code == "missing_required_dependency" {
        format!("{} requires {target}", source.project_id)
    } else {
        format!("{} declares {target} incompatible", source.project_id)
    };
    CompatibilityIssue::new(code, IssueSeverity::Blocking, summary)
        .with_subjects([
            project_key(source.provider, &source.project_id),
            target.clone(),
        ])
        .with_suggested_actions([
            SuggestedAction::new(if code == "missing_required_dependency" {
                "resolve_required_dependency"
            } else {
                "remove_conflicting_project"
            })
            .with_target(target),
        ])
}

fn project_key(provider: ProviderId, project_id: &str) -> String {
    format!("{}:{project_id}", provider_slug(provider))
}
