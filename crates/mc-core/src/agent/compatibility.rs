use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus {
    Healthy,
    Warning,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Info,
    Warning,
    Blocking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SuggestedAction {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl SuggestedAction {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            target: None,
            value: None,
        }
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CompatibilityIssue {
    pub code: String,
    pub severity: IssueSeverity,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_actions: Vec<SuggestedAction>,
}

impl CompatibilityIssue {
    pub fn new(
        code: impl Into<String>,
        severity: IssueSeverity,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity,
            summary: summary.into(),
            subjects: Vec::new(),
            evidence: Vec::new(),
            suggested_actions: Vec::new(),
        }
    }

    pub fn with_subjects(mut self, subjects: impl IntoIterator<Item = String>) -> Self {
        self.subjects = subjects.into_iter().collect();
        self
    }

    pub fn with_evidence(mut self, evidence: impl IntoIterator<Item = String>) -> Self {
        self.evidence = evidence.into_iter().collect();
        self
    }

    pub fn with_suggested_actions(
        mut self,
        actions: impl IntoIterator<Item = SuggestedAction>,
    ) -> Self {
        self.suggested_actions = actions.into_iter().collect();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CompatibilityReport {
    pub status: CompatibilityStatus,
    pub issues: Vec<CompatibilityIssue>,
}

impl CompatibilityReport {
    pub fn from_issues(issues: Vec<CompatibilityIssue>) -> Self {
        let status = if issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Blocking)
        {
            CompatibilityStatus::Blocked
        } else if issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Warning)
        {
            CompatibilityStatus::Warning
        } else {
            CompatibilityStatus::Healthy
        };
        Self { status, issues }
    }

    pub fn is_blocked(&self) -> bool {
        self.status == CompatibilityStatus::Blocked
    }
}

impl Default for CompatibilityReport {
    fn default() -> Self {
        Self::from_issues(Vec::new())
    }
}
