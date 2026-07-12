use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent::compatibility::{
    CompatibilityIssue, CompatibilityReport, IssueSeverity, SuggestedAction,
};
use crate::instance::{list_instances, list_mods, Instance, ModInfo};
use crate::paths::GamePaths;

use super::ChatToolError;

const MAX_LOG_BYTES: u64 = 512 * 1024;
const MAX_LOG_LINES: usize = 200;

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct DiagnoseInstanceArgs {
    #[serde(default)]
    pub include_log_tail: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct InstanceDiagnosticSummary {
    pub id: String,
    pub name: String,
    pub mc_version: String,
    pub loader: String,
    pub memory_mb: u32,
    pub recommended_memory_mb: u32,
    pub mod_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct DiagnoseInstanceOutput {
    pub instance: InstanceDiagnosticSummary,
    pub report: CompatibilityReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_tail: Option<String>,
}

pub async fn tool_diagnose_instance(
    paths: &GamePaths,
    instance_id: &str,
    args: DiagnoseInstanceArgs,
) -> Result<DiagnoseInstanceOutput, ChatToolError> {
    diagnose_instance_with_total_memory(
        paths,
        instance_id,
        args,
        crate::system::system_total_mem_mb(),
    )
}

pub(crate) fn diagnose_instance_with_total_memory(
    paths: &GamePaths,
    instance_id: &str,
    args: DiagnoseInstanceArgs,
    total_memory_mb: u64,
) -> Result<DiagnoseInstanceOutput, ChatToolError> {
    let summary = list_instances(paths)
        .into_iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| ChatToolError::new(format!("instance not found: {instance_id}")))?;
    if !summary.installed {
        return Err(ChatToolError::new(format!(
            "instance is not installed: {instance_id}"
        )));
    }

    let instance = Instance::new(instance_id, paths.root());
    let config = instance.load_config()?;
    let mods: Vec<_> = list_mods(&instance)
        .into_iter()
        .filter(|mod_info| mod_info.enabled)
        .collect();
    let recommended_memory_mb = crate::system::suggest_memory_mb(total_memory_mb, mods.len());
    let mut issues = Vec::new();

    append_duplicate_mod_issues(&mods, &mut issues);
    append_loader_mismatch_issues(summary.loader.as_str(), &mods, &mut issues);
    if recommended_memory_mb > 0 && config.memory_mb < recommended_memory_mb {
        issues.push(
            CompatibilityIssue::new(
                "memory_below_recommendation",
                IssueSeverity::Warning,
                format!(
                    "Instance memory is {} MiB; {} MiB is recommended for this mod count",
                    config.memory_mb, recommended_memory_mb
                ),
            )
            .with_evidence([format!("enabled_mod_count={}", mods.len())])
            .with_suggested_actions([
                SuggestedAction::new("set_memory").with_value(recommended_memory_mb.to_string())
            ]),
        );
    }

    let analyzed_log_tail = find_instance_log(&instance).and_then(|path| read_log_tail(&path));
    if let Some(analysis) = analyzed_log_tail
        .as_deref()
        .and_then(crate::diagnostics::analyze)
    {
        let evidence = analysis
            .matched
            .into_iter()
            .chain([format!("category={}", analysis.category.slug())])
            .collect::<Vec<_>>();
        let actions = analysis.suggestions.into_iter().map(|suggestion| {
            SuggestedAction::new("review_crash_suggestion").with_value(suggestion)
        });
        issues.push(
            CompatibilityIssue::new("last_launch_crash", IssueSeverity::Warning, analysis.reason)
                .with_evidence(evidence)
                .with_suggested_actions(actions),
        );
    }

    Ok(DiagnoseInstanceOutput {
        instance: InstanceDiagnosticSummary {
            id: summary.id,
            name: summary.name,
            mc_version: summary.mc_version,
            loader: summary.loader.as_str().to_string(),
            memory_mb: config.memory_mb,
            recommended_memory_mb,
            mod_count: mods.len(),
        },
        report: CompatibilityReport::from_issues(issues),
        log_tail: args.include_log_tail.then_some(analyzed_log_tail).flatten(),
    })
}

fn append_duplicate_mod_issues(mods: &[ModInfo], issues: &mut Vec<CompatibilityIssue>) {
    let mut by_id: BTreeMap<String, Vec<&ModInfo>> = BTreeMap::new();
    for mod_info in mods {
        if let Some(mod_id) = mod_info.mod_id.as_deref().filter(|id| !id.is_empty()) {
            by_id
                .entry(mod_id.to_ascii_lowercase())
                .or_default()
                .push(mod_info);
        }
    }
    for (mod_id, duplicates) in by_id.into_iter().filter(|(_, entries)| entries.len() > 1) {
        let subjects = duplicates
            .iter()
            .map(|entry| entry.file_name.clone())
            .collect::<Vec<_>>();
        issues.push(
            CompatibilityIssue::new(
                "duplicate_mod_id",
                IssueSeverity::Blocking,
                format!("Multiple enabled mod files declare id {mod_id}"),
            )
            .with_subjects(subjects)
            .with_suggested_actions([
                SuggestedAction::new("review_duplicate_mods").with_target(mod_id)
            ]),
        );
    }
}

fn append_loader_mismatch_issues(
    instance_loader: &str,
    mods: &[ModInfo],
    issues: &mut Vec<CompatibilityIssue>,
) {
    for mod_info in mods {
        if mod_loader_matches(instance_loader, &mod_info.loader) {
            continue;
        }
        issues.push(
            CompatibilityIssue::new(
                "mod_loader_mismatch",
                IssueSeverity::Blocking,
                format!(
                    "{} targets {} but the instance uses {}",
                    mod_info.file_name, mod_info.loader, instance_loader
                ),
            )
            .with_subjects([mod_info.file_name.clone()])
            .with_suggested_actions([SuggestedAction::new("set_mod_enabled")
                .with_target(mod_info.file_name.clone())
                .with_value("false")]),
        );
    }
}

fn mod_loader_matches(instance_loader: &str, mod_loader: &str) -> bool {
    match (instance_loader, mod_loader) {
        (_, "unknown") => true,
        ("quilt", "fabric" | "quilt") => true,
        (instance, declared) => instance == declared,
    }
}

fn find_instance_log(instance: &Instance) -> Option<PathBuf> {
    let latest = instance.game_dir().join("logs/latest.log");
    if latest.is_file() {
        return Some(latest);
    }

    std::fs::read_dir(instance.game_dir().join("crash-reports"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .max_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
        })
}

fn read_log_tail(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(MAX_LOG_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::with_capacity((length - start) as usize);
    file.read_to_end(&mut bytes).ok()?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        text = text
            .split_once('\n')
            .map(|(_, rest)| rest.to_string())
            .unwrap_or_default();
    }
    let lines = text.lines().collect::<Vec<_>>();
    let tail_start = lines.len().saturating_sub(MAX_LOG_LINES);
    Some(lines[tail_start..].join("\n"))
}
