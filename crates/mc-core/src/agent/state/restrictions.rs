use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
pub struct BuildRestrictions {
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version_requirement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    #[serde(default)]
    pub feature_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default)]
    pub history: Vec<BuildRestrictionChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildRestrictionsLlmView {
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version_requirement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    #[serde(default)]
    pub feature_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl BuildRestrictions {
    pub fn llm_view(&self) -> BuildRestrictionsLlmView {
        BuildRestrictionsLlmView {
            revision: self.revision,
            minecraft_version: self.minecraft_version.clone(),
            minecraft_version_requirement: self.minecraft_version_requirement.clone(),
            loader: self.loader.clone(),
            feature_tags: self.feature_tags.clone(),
            notes: self.notes.clone(),
        }
    }

    /// Apply a restriction patch under optimistic concurrency and return the
    /// resulting view.
    ///
    /// This is the single authority for mutating build restrictions. It rejects
    /// the write when `base_revision` no longer matches the current revision
    /// (same `Err` shape the caller relied on before), then runs ONE
    /// normalization pass: an invalid `minecraft_version` is dropped *with a
    /// warning* (not silently), the version requirement falls back to the
    /// concrete version, the loader is whitelisted (warning on an unsupported
    /// one), and feature tags are trimmed, lowercased, capped, then deduped.
    /// The normalized patch is stored, the revision is bumped, and a history
    /// entry is appended. `missing_fields`/`warnings` are derived from the
    /// stored result. The free `update_build_restrictions` wrapper and every
    /// replan route here, so the two normalization passes that used to drift
    /// can no longer disagree.
    pub(in crate::agent) fn try_apply(
        &mut self,
        base_revision: u64,
        patch: BuildRestrictionPatch,
        source: BuildRestrictionChangeSource,
        summary: impl Into<String>,
    ) -> Result<UpdateBuildRestrictionsOutput> {
        if base_revision != self.revision {
            return Err(CoreError::other(format!(
                "update_build_restrictions revision mismatch: expected {}, got {}",
                self.revision, base_revision
            )));
        }

        let mut warnings = Vec::new();
        let mut patch_minecraft_version = None;
        let minecraft_version = match patch
            .minecraft_version
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(s) if is_minecraft_version(s) => {
                let version = s.to_string();
                patch_minecraft_version = Some(version.clone());
                Some(version)
            }
            Some(s) => {
                warnings.push(format!("ignored invalid minecraft_version: {s}"));
                self.minecraft_version.clone()
            }
            None => self.minecraft_version.clone(),
        };
        let minecraft_version_requirement = patch
            .minecraft_version_requirement
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| patch_minecraft_version.clone())
            .or_else(|| self.minecraft_version_requirement.clone());
        let loader = match patch
            .loader
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(raw) => normalize_loader(raw).or_else(|| {
                warnings.push("ignored unsupported loader".to_string());
                self.loader.clone()
            }),
            None => self.loader.clone(),
        };
        let notes = patch
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| self.notes.clone());
        let normalized = BuildRestrictionPatch {
            minecraft_version,
            minecraft_version_requirement,
            loader,
            feature_tags: normalize_feature_tags(patch.feature_tags),
            notes,
        };

        self.minecraft_version = normalized.minecraft_version.clone();
        self.minecraft_version_requirement = normalized.minecraft_version_requirement.clone();
        self.loader = normalized.loader.clone();
        self.feature_tags = normalized.feature_tags.clone();
        self.notes = normalized.notes.clone();
        self.revision += 1;
        self.history.push(BuildRestrictionChange {
            revision: self.revision,
            source,
            patch: normalized,
            summary: summary.into(),
        });

        Ok(UpdateBuildRestrictionsOutput {
            missing_fields: missing_restriction_fields(self),
            restrictions: self.clone(),
            warnings,
        })
    }

    /// Project the current restrictions into an update output *without* applying
    /// a patch, deriving `missing_fields` exactly as [`Self::try_apply`] does.
    /// Gates that must surface the existing restrictions plus a contextual
    /// warning (customization/execution blocks) use this so they never re-derive
    /// the output shape by hand.
    pub(in crate::agent) fn as_update_output(
        &self,
        warnings: Vec<String>,
    ) -> UpdateBuildRestrictionsOutput {
        UpdateBuildRestrictionsOutput {
            missing_fields: missing_restriction_fields(self),
            restrictions: self.clone(),
            warnings,
        }
    }
}

/// Which hard-requirement fields are still unset. Kept next to [`BuildRestrictions::try_apply`]
/// since both the applied output and the projected output derive from it.
fn missing_restriction_fields(restrictions: &BuildRestrictions) -> Vec<String> {
    let mut missing = Vec::new();
    if restrictions.minecraft_version.is_none() {
        missing.push("minecraft_version".to_string());
    }
    if restrictions.loader.is_none() {
        missing.push("loader".to_string());
    }
    missing
}

/// A permissive "looks like a Minecraft release" check (`1.x[.y[.z]]`).
pub(in crate::agent) fn is_minecraft_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() >= 2
        && parts.len() <= 4
        && parts.first() == Some(&"1")
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Whitelist a loader name to its canonical lowercase form, or `None` if
/// unsupported.
pub(in crate::agent) fn normalize_loader(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "fabric" => Some("fabric".to_string()),
        "forge" => Some("forge".to_string()),
        "neoforge" | "neo forge" => Some("neoforge".to_string()),
        "quilt" => Some("quilt".to_string()),
        _ => None,
    }
}

/// Trim + lowercase feature tags, cap at eight, then dedupe (first occurrence
/// wins). The cap is applied before the dedupe so it matches the pre-refactor
/// authoritative pass exactly.
fn normalize_feature_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.into_iter()
        .map(|tag| tag.trim().to_ascii_lowercase())
        .filter(|tag| !tag.is_empty())
        .take(8)
        .filter(|tag| seen.insert(tag.clone()))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildRestrictionPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version_requirement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    #[serde(default)]
    pub feature_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateBuildRestrictionsInput {
    pub base_revision: u64,
    pub patch: BuildRestrictionPatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateBuildRestrictionsOutput {
    pub restrictions: BuildRestrictions,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildRestrictionChange {
    pub revision: u64,
    pub source: BuildRestrictionChangeSource,
    pub patch: BuildRestrictionPatch,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BuildRestrictionChangeSource {
    InitialPrompt,
    UserRevise,
    UiEdit,
}
