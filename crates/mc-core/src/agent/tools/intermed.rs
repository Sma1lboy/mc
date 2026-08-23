use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;
use uuid::Uuid;

use crate::agent::compatibility::{CompatibilityIssue, IssueSeverity, SuggestedAction};
use crate::instance::{Instance, ModInfo};

use super::ChatToolError;

const SCAN_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_REPORT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_EXPOSED_FINDINGS: usize = 20;

#[derive(Debug, Clone)]
pub struct IntermedConfig {
    pub binary: PathBuf,
    pub cache_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntermedScanStatus {
    Healthy,
    Warning,
    Blocked,
    Incomplete,
}

#[derive(Debug)]
pub(crate) struct IntermedScan {
    pub status: IntermedScanStatus,
    pub tool_version: String,
    pub schema: String,
    pub findings_total: usize,
    pub issues: Vec<CompatibilityIssue>,
    blocking_components: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct DoctorReport {
    schema: String,
    tool_version: String,
    #[serde(default)]
    summary: DoctorSummary,
    #[serde(default)]
    findings: Vec<DoctorFinding>,
    #[serde(default)]
    collectors: Vec<CollectorReport>,
    #[serde(default, deserialize_with = "null_default")]
    operational_errors: Vec<OperationalError>,
}

#[derive(Debug, Default, Deserialize)]
struct DoctorSummary {
    #[serde(default)]
    fatal: usize,
    #[serde(default)]
    error: usize,
    #[serde(default)]
    warn: usize,
    #[serde(default)]
    total: usize,
    #[serde(default)]
    incomplete_analysis: usize,
}

#[derive(Debug, Deserialize)]
struct DoctorFinding {
    id: String,
    rule_id: String,
    severity: String,
    #[serde(default)]
    category: String,
    title: String,
    explanation: String,
    #[serde(default)]
    confidence: f32,
    #[serde(default)]
    affected_components: Vec<String>,
    #[serde(default)]
    fix_candidates: Vec<FixCandidate>,
}

#[derive(Debug, Deserialize)]
struct FixCandidate {
    description: String,
    #[serde(default)]
    confidence: f32,
}

#[derive(Debug, Deserialize)]
struct CollectorReport {
    id: String,
    status: String,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct OperationalError {
    stage: String,
    component: String,
    message: String,
}

pub(crate) async fn scan_instance(
    config: &IntermedConfig,
    instance: &Instance,
) -> Result<IntermedScan, ChatToolError> {
    if !config.binary.is_file() {
        return Err(ChatToolError::new(
            "InterMed scanner is not bundled. Reinstall or repair the launcher before retrying static diagnosis.",
        ));
    }

    let reports_dir = config.cache_dir.join("reports");
    tokio::fs::create_dir_all(&reports_dir)
        .await
        .map_err(|error| {
            ChatToolError::new(format!("cannot create InterMed report directory: {error}"))
        })?;
    let report_path = reports_dir.join(format!("{}.json", Uuid::new_v4()));
    let scan_cache = config.cache_dir.join("cache");

    let mut command = Command::new(&config.binary);
    command
        .kill_on_drop(true)
        .arg("--quiet")
        .arg("doctor")
        .arg(instance.game_dir())
        .arg("--mods-dir")
        .arg(instance.mods_dir())
        .arg("--json")
        .arg(&report_path)
        .arg("--report-schema")
        .arg("v2")
        .arg("--exit-zero")
        .arg("--jobs")
        .arg("2")
        .arg("--metadata-level")
        .arg("enriched")
        .arg("--resource-level")
        .arg("semantic")
        .arg("--mixin-level")
        .arg("standard")
        .arg("--cache-dir")
        .arg(scan_cache);

    let output = match tokio::time::timeout(SCAN_TIMEOUT, command.output()).await {
        Ok(result) => result.map_err(|error| {
            ChatToolError::new(format!(
                "failed to start the bundled InterMed scanner: {error}"
            ))
        })?,
        Err(_) => {
            let _ = tokio::fs::remove_file(&report_path).await;
            return Err(ChatToolError::new(
                "InterMed scan timed out after 120 seconds. Retry after reducing the active Mod set.",
            ));
        }
    };

    if !output.status.success() {
        let _ = tokio::fs::remove_file(&report_path).await;
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ChatToolError::new(format!(
            "InterMed scan failed operationally: {}",
            truncate_text(&redact_path(stderr.trim(), &instance.game_dir()), 600)
        )));
    }

    let bytes = read_bounded_report(&report_path).await;
    let _ = tokio::fs::remove_file(&report_path).await;
    let bytes = bytes?;
    parse_report(&bytes, &instance.game_dir())
}

async fn read_bounded_report(path: &Path) -> Result<Vec<u8>, ChatToolError> {
    let metadata = tokio::fs::metadata(path).await.map_err(|error| {
        ChatToolError::new(format!("InterMed did not produce its JSON report: {error}"))
    })?;
    if metadata.len() > MAX_REPORT_BYTES {
        return Err(ChatToolError::new(format!(
            "InterMed JSON report exceeded the {} MiB safety limit",
            MAX_REPORT_BYTES / 1024 / 1024
        )));
    }
    tokio::fs::read(path)
        .await
        .map_err(|error| ChatToolError::new(format!("cannot read InterMed JSON report: {error}")))
}

fn parse_report(bytes: &[u8], instance_dir: &Path) -> Result<IntermedScan, ChatToolError> {
    let report: DoctorReport = serde_json::from_slice(bytes)
        .map_err(|error| ChatToolError::new(format!("invalid InterMed JSON report: {error}")))?;
    if report.schema != "intermed-doctor-report-v2" {
        return Err(ChatToolError::new(format!(
            "unsupported InterMed report schema {}; expected intermed-doctor-report-v2",
            report.schema
        )));
    }

    let incomplete_collector = report
        .collectors
        .iter()
        .find(|collector| matches!(collector.status.as_str(), "failed" | "error" | "incomplete"));
    let status = if !report.operational_errors.is_empty()
        || report.summary.incomplete_analysis > 0
        || incomplete_collector.is_some()
    {
        IntermedScanStatus::Incomplete
    } else if report.summary.fatal > 0 || report.summary.error > 0 {
        IntermedScanStatus::Blocked
    } else if report.summary.warn > 0 {
        IntermedScanStatus::Warning
    } else {
        IntermedScanStatus::Healthy
    };

    let mut surfaced = report.findings.iter().collect::<Vec<_>>();
    surfaced.sort_by_key(|finding| severity_rank(&finding.severity));
    let mut issues = surfaced
        .into_iter()
        .filter(|finding| finding.severity != "info" || issues_are_sparse(&report))
        .take(MAX_EXPOSED_FINDINGS)
        .map(|finding| normalize_finding(finding, instance_dir))
        .collect::<Vec<_>>();
    if status == IntermedScanStatus::Blocked
        && !issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Blocking)
    {
        issues.push(CompatibilityIssue::new(
            "intermed_blocked_without_details",
            IssueSeverity::Blocking,
            "InterMed reported a blocking static compatibility result without actionable details",
        ));
    }
    if status == IntermedScanStatus::Incomplete {
        issues.push(incomplete_issue(
            &report,
            incomplete_collector,
            instance_dir,
        ));
    }

    let blocking_components = report
        .findings
        .iter()
        .filter(|finding| matches!(finding.severity.as_str(), "fatal" | "error"))
        .map(|finding| finding.affected_components.clone())
        .collect();
    let findings_total = report.summary.total.max(report.findings.len());
    Ok(IntermedScan {
        status,
        tool_version: report.tool_version,
        schema: report.schema,
        findings_total,
        issues,
        blocking_components,
    })
}

fn severity_rank(severity: &str) -> u8 {
    match severity {
        "fatal" => 0,
        "error" => 1,
        "warn" => 2,
        "note" => 3,
        _ => 4,
    }
}

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

fn issues_are_sparse(report: &DoctorReport) -> bool {
    report.findings.len() <= MAX_EXPOSED_FINDINGS
}

fn normalize_finding(finding: &DoctorFinding, instance_dir: &Path) -> CompatibilityIssue {
    let severity = match finding.severity.as_str() {
        "fatal" | "error" => IssueSeverity::Blocking,
        "warn" => IssueSeverity::Warning,
        _ => IssueSeverity::Info,
    };
    let subjects = finding
        .affected_components
        .iter()
        .map(|component| redact_path(component, instance_dir))
        .take(12)
        .collect::<Vec<_>>();
    let mut evidence = vec![format!(
        "scanner=intermed finding={} category={} confidence={:.2}",
        finding.id, finding.category, finding.confidence
    )];
    evidence.push(truncate_text(
        &redact_path(&finding.explanation, instance_dir),
        1_000,
    ));
    let actions = finding.fix_candidates.iter().take(4).map(|candidate| {
        SuggestedAction::new("review_intermed_fix").with_value(format!(
            "{} (confidence {:.2})",
            truncate_text(&redact_path(&candidate.description, instance_dir), 400),
            candidate.confidence
        ))
    });
    CompatibilityIssue::new(
        format!("intermed:{}", finding.rule_id),
        severity,
        truncate_text(&redact_path(&finding.title, instance_dir), 300),
    )
    .with_subjects(subjects)
    .with_evidence(evidence)
    .with_suggested_actions(actions)
}

fn incomplete_issue(
    report: &DoctorReport,
    collector: Option<&CollectorReport>,
    instance_dir: &Path,
) -> CompatibilityIssue {
    let mut evidence = Vec::new();
    for error in report.operational_errors.iter().take(4) {
        evidence.push(format!(
            "{}:{}: {}",
            error.stage,
            error.component,
            truncate_text(&redact_path(&error.message, instance_dir), 400)
        ));
    }
    if let Some(collector) = collector {
        evidence.push(format!(
            "collector {} status={}: {}",
            collector.id,
            collector.status,
            truncate_text(&redact_path(&collector.message, instance_dir), 400)
        ));
    }
    CompatibilityIssue::new(
        "intermed_incomplete",
        IssueSeverity::Blocking,
        "InterMed could not complete the required static analysis",
    )
    .with_evidence(evidence)
}

pub(crate) fn quarantine_candidates(scan: &IntermedScan, mods: &[ModInfo]) -> Vec<String> {
    let mut candidates = BTreeSet::new();
    for components in &scan.blocking_components {
        let matched = mods
            .iter()
            .filter(|mod_info| {
                mod_info.enabled
                    && components
                        .iter()
                        .any(|component| component_matches_mod(component, mod_info))
            })
            .collect::<Vec<_>>();
        if matched.len() == 1 {
            candidates.insert(matched[0].file_name.clone());
        }
    }
    candidates.into_iter().collect()
}

fn component_matches_mod(component: &str, mod_info: &ModInfo) -> bool {
    let component = component.trim().to_ascii_lowercase();
    if component == mod_info.file_name.to_ascii_lowercase() {
        return true;
    }
    let Some(mod_id) = mod_info.mod_id.as_deref() else {
        return false;
    };
    let mod_id = mod_id.to_ascii_lowercase();
    component == mod_id
        || component.strip_prefix("mod:") == Some(mod_id.as_str())
        || component.strip_prefix("mod/") == Some(mod_id.as_str())
}

fn redact_path(value: &str, instance_dir: &Path) -> String {
    let root = instance_dir.to_string_lossy();
    let redacted = value.replace(root.as_ref(), "<instance>");
    #[cfg(windows)]
    let redacted = redacted.replace(&root.replace('\\', "/"), "<instance>");
    redacted
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mod_info(file_name: &str, mod_id: &str) -> ModInfo {
        ModInfo {
            file_name: file_name.into(),
            name: mod_id.into(),
            version: Some("1.0.0".into()),
            enabled: true,
            loader: "fabric".into(),
            mod_id: Some(mod_id.into()),
            authors: Vec::new(),
            description: None,
            size: 0,
        }
    }

    #[test]
    fn parses_v2_and_redacts_instance_paths() {
        let json = br#"{
          "schema":"intermed-doctor-report-v2","tool_version":"0.1.7-alpha",
          "summary":{"fatal":0,"error":1,"warn":0,"total":1,"incomplete_analysis":0},
          "findings":[{"id":"missing:cloth","rule_id":"deps.missing","severity":"error",
            "category":"dependency","title":"Missing dependency","explanation":"Found in /games/pack/mods/a.jar",
            "confidence":0.99,"affected_components":["mod:example"],"fix_candidates":[]}],
          "collectors":[{"id":"metadata","status":"active","message":"ok"}],
          "operational_errors":null
        }"#;
        let scan = parse_report(json, Path::new("/games/pack")).unwrap();
        assert_eq!(scan.status, IntermedScanStatus::Blocked);
        assert_eq!(scan.issues[0].severity, IssueSeverity::Blocking);
        assert!(scan.issues[0].evidence[1].contains("<instance>/mods/a.jar"));
    }

    #[test]
    fn incomplete_report_fails_closed() {
        let json = br#"{
          "schema":"intermed-doctor-report-v2","tool_version":"0.1.7-alpha",
          "summary":{"total":0,"incomplete_analysis":1},"findings":[],
          "collectors":[{"id":"metadata","status":"incomplete","message":"truncated"}]
        }"#;
        let scan = parse_report(json, Path::new("/pack")).unwrap();
        assert_eq!(scan.status, IntermedScanStatus::Incomplete);
        assert_eq!(scan.issues[0].code, "intermed_incomplete");
    }

    #[test]
    fn quarantines_only_a_uniquely_implicated_mod() {
        let mut scan = parse_report(
            br#"{"schema":"intermed-doctor-report-v2","tool_version":"0.1.7-alpha",
              "summary":{"error":1,"total":1},
              "findings":[{"id":"x","rule_id":"x","severity":"error","category":"dependency",
                "title":"x","explanation":"x","affected_components":["mod:alpha"],"fix_candidates":[]}]}"#,
            Path::new("/pack"),
        )
        .unwrap();
        let mods = vec![mod_info("alpha.jar", "alpha"), mod_info("beta.jar", "beta")];
        assert_eq!(quarantine_candidates(&scan, &mods), vec!["alpha.jar"]);

        scan.blocking_components = vec![vec!["mod:alpha".into(), "mod:beta".into()]];
        assert!(quarantine_candidates(&scan, &mods).is_empty());
    }
}
