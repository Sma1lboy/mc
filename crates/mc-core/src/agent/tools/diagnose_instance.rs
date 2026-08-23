use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent::compatibility::{
    CompatibilityIssue, CompatibilityReport, IssueSeverity, SuggestedAction,
};
use crate::instance::{list_instances, list_mods, Instance, ModInfo};
use crate::paths::GamePaths;

use super::intermed::{quarantine_candidates, scan_instance, IntermedConfig, IntermedScanStatus};
use super::ChatToolError;

const MAX_LOG_BYTES: u64 = 512 * 1024;
const MAX_LOG_LINES: usize = 200;
const MAX_REMEDIATION_PASSES: u8 = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisMode {
    #[default]
    Inspect,
    Remediate,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct DiagnoseInstanceArgs {
    #[serde(default)]
    pub include_log_tail: bool,
    #[serde(default)]
    pub mode: DiagnosisMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum StaticAnalysisStatus {
    Healthy,
    Warning,
    Blocked,
    Incomplete,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct StaticAnalysisSummary {
    pub status: StaticAnalysisStatus,
    pub scanner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub findings_total: usize,
    pub passes: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CompatibilityRemediationSummary {
    pub passes: u8,
    pub static_compatible: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_mods: Vec<String>,
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
    pub static_analysis: Option<StaticAnalysisSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<CompatibilityRemediationSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_tail: Option<String>,
}

pub async fn tool_diagnose_instance(
    paths: &GamePaths,
    instance_id: &str,
    args: DiagnoseInstanceArgs,
) -> Result<DiagnoseInstanceOutput, ChatToolError> {
    if args.mode == DiagnosisMode::Remediate {
        return Err(ChatToolError::new(
            "remediation requires the desktop's bundled InterMed scanner",
        ));
    }
    diagnose_instance_with_total_memory(
        paths,
        instance_id,
        args,
        crate::system::system_total_mem_mb(),
    )
}

pub async fn tool_diagnose_instance_with_intermed(
    paths: &GamePaths,
    instance_id: &str,
    args: DiagnoseInstanceArgs,
    intermed: Option<&IntermedConfig>,
) -> Result<DiagnoseInstanceOutput, ChatToolError> {
    diagnose_instance_with_intermed_and_memory(
        paths,
        instance_id,
        args,
        intermed,
        crate::system::system_total_mem_mb(),
    )
    .await
}

async fn diagnose_instance_with_intermed_and_memory(
    paths: &GamePaths,
    instance_id: &str,
    args: DiagnoseInstanceArgs,
    intermed: Option<&IntermedConfig>,
    total_memory_mb: u64,
) -> Result<DiagnoseInstanceOutput, ChatToolError> {
    let mode = args.mode;
    let mut disabled_mods = Vec::new();
    let instance = Instance::new(instance_id, paths.root());

    for pass in 1..=MAX_REMEDIATION_PASSES {
        let mut output =
            diagnose_instance_with_total_memory(paths, instance_id, args.clone(), total_memory_mb)?;
        let local_issues = output.report.issues.clone();
        let Some(config) = intermed else {
            append_scanner_failure(
                &mut output,
                pass,
                "InterMed is not bundled for this platform",
            );
            output.remediation = remediation_summary(mode, pass, disabled_mods, &output);
            return Ok(output);
        };
        let scan = match scan_instance(config, &instance).await {
            Ok(scan) => scan,
            Err(error) => {
                append_scanner_failure(&mut output, pass, &error.to_string());
                output.remediation = remediation_summary(mode, pass, disabled_mods, &output);
                return Ok(output);
            }
        };
        output.report.issues.extend(scan.issues.clone());
        output.report = CompatibilityReport::from_issues(output.report.issues);
        output.static_analysis = Some(StaticAnalysisSummary {
            status: static_status(scan.status),
            scanner: "intermed".into(),
            tool_version: Some(scan.tool_version.clone()),
            schema: Some(scan.schema.clone()),
            findings_total: scan.findings_total,
            passes: pass,
            message: None,
        });

        if mode == DiagnosisMode::Inspect
            || scan.status == IntermedScanStatus::Incomplete
            || !output.report.is_blocked()
        {
            output.remediation = remediation_summary(mode, pass, disabled_mods, &output);
            return Ok(output);
        }
        if pass == MAX_REMEDIATION_PASSES {
            output.remediation = remediation_summary(mode, pass, disabled_mods, &output);
            return Ok(output);
        }

        let mods = list_mods(&instance);
        let mut candidates = local_quarantine_candidates(&local_issues);
        candidates.extend(quarantine_candidates(&scan, &mods));
        candidates.sort();
        candidates.dedup();
        candidates.retain(|file_name| {
            mods.iter()
                .any(|mod_info| mod_info.enabled && mod_info.file_name == *file_name)
        });
        if candidates.is_empty() {
            output.remediation = remediation_summary(mode, pass, disabled_mods, &output);
            return Ok(output);
        }
        for file_name in candidates {
            crate::instance::mods::set_mod_enabled(&instance, &file_name, false)?;
            disabled_mods.push(file_name);
        }
    }
    unreachable!("bounded remediation loop always returns")
}

fn append_scanner_failure(output: &mut DiagnoseInstanceOutput, pass: u8, message: &str) {
    output.report.issues.push(
        CompatibilityIssue::new(
            "intermed_unavailable",
            IssueSeverity::Blocking,
            "InterMed static analysis is unavailable",
        )
        .with_evidence([message.to_string()]),
    );
    output.report = CompatibilityReport::from_issues(output.report.issues.clone());
    output.static_analysis = Some(StaticAnalysisSummary {
        status: StaticAnalysisStatus::Unavailable,
        scanner: "intermed".into(),
        tool_version: None,
        schema: None,
        findings_total: 0,
        passes: pass,
        message: Some(message.to_string()),
    });
}

fn remediation_summary(
    mode: DiagnosisMode,
    passes: u8,
    disabled_mods: Vec<String>,
    output: &DiagnoseInstanceOutput,
) -> Option<CompatibilityRemediationSummary> {
    (mode == DiagnosisMode::Remediate).then(|| CompatibilityRemediationSummary {
        passes,
        static_compatible: !output.report.is_blocked()
            && output.static_analysis.as_ref().is_some_and(|analysis| {
                matches!(
                    analysis.status,
                    StaticAnalysisStatus::Healthy | StaticAnalysisStatus::Warning
                )
            }),
        disabled_mods,
    })
}

fn static_status(status: IntermedScanStatus) -> StaticAnalysisStatus {
    match status {
        IntermedScanStatus::Healthy => StaticAnalysisStatus::Healthy,
        IntermedScanStatus::Warning => StaticAnalysisStatus::Warning,
        IntermedScanStatus::Blocked => StaticAnalysisStatus::Blocked,
        IntermedScanStatus::Incomplete => StaticAnalysisStatus::Incomplete,
    }
}

fn local_quarantine_candidates(issues: &[CompatibilityIssue]) -> Vec<String> {
    let mut files = Vec::new();
    for issue in issues {
        match issue.code.as_str() {
            "mod_loader_mismatch" => files.extend(issue.subjects.iter().cloned()),
            "duplicate_mod_id" => {
                let mut duplicates = issue.subjects.clone();
                duplicates.sort();
                files.extend(duplicates.into_iter().skip(1));
            }
            _ => {}
        }
    }
    files
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
        static_analysis: None,
        remediation: None,
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
