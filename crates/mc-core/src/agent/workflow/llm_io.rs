use super::*;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum AgentIntentOutputKind {
    BuildModpack,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(super) struct AgentIntentOutput {
    pub intent: AgentIntentOutputKind,
    pub confidence: f32,
    pub rationale: String,
}

impl AgentIntentOutput {
    pub(super) fn into_agent_intent(self) -> AgentIntent {
        AgentIntent {
            kind: match self.intent {
                AgentIntentOutputKind::BuildModpack => AgentIntentKind::BuildModpack,
                AgentIntentOutputKind::Unknown => AgentIntentKind::Unknown,
            },
            confidence: self.confidence.clamp(0.0, 1.0),
            rationale: (!self.rationale.trim().is_empty()).then_some(self.rationale),
        }
    }
}

#[cfg(test)]
pub(super) fn parse_intent_response(text: &str) -> Option<AgentIntent> {
    let value = parse_first_json_object(text)?;
    let raw = value.get("intent")?.as_str()?;
    let kind = intent_kind(raw);
    let confidence = value
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5)
        .clamp(0.0, 1.0) as f32;
    let rationale = value
        .get("rationale")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToOwned::to_owned);
    Some(AgentIntent {
        kind,
        confidence,
        rationale,
    })
}

pub(super) fn parse_first_json_object(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(value);
    }
    let start = trimmed.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in trimmed[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    let end = start + offset;
                    return serde_json::from_str(&trimmed[start..=end]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
fn intent_kind(value: &str) -> AgentIntentKind {
    match value.trim().to_ascii_lowercase().as_str() {
        "build_modpack" | "modpack_build" | "create_modpack" => AgentIntentKind::BuildModpack,
        _ => AgentIntentKind::Unknown,
    }
}

#[derive(Debug, Clone)]
pub(super) enum ApprovalRoute {
    Decision(UserDecision),
    NeedsClarification { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum ApprovalDecisionOutputKind {
    Approve,
    Revise,
    Cancel,
    NeedsClarification,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(super) struct ApprovalRouteOutput {
    pub decision: ApprovalDecisionOutputKind,
    pub selected_option_id: Option<String>,
    pub message: Option<String>,
    pub rationale: String,
}

impl ApprovalRouteOutput {
    pub(super) fn into_route(self, approval: &ApprovalRequest) -> Result<ApprovalRoute> {
        let text = serde_json::to_string(&self).map_err(|source| CoreError::Parse {
            what: "approval route output".into(),
            source,
        })?;
        parse_approval_route_response(&text, approval)
    }
}

pub(super) fn parse_approval_route_response(
    text: &str,
    approval: &ApprovalRequest,
) -> Result<ApprovalRoute> {
    let value = parse_first_json_object(text).ok_or_else(|| {
        CoreError::other(format!(
            "could not parse approval decision JSON from model output: {text}"
        ))
    })?;
    let decision = value
        .get("decision")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let raw_selected_option_id = value
        .get("selected_option_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let raw_message = value
        .get("message")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let (kind, selected_option_id, message) = match decision.as_str() {
        "approve" => (UserDecisionKind::Approve, raw_selected_option_id, None),
        "revise" => (UserDecisionKind::Revise, None, raw_message),
        "cancel" => (UserDecisionKind::Cancel, None, None),
        "needs_clarification" => {
            let rationale = value
                .get("rationale")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("approval decision needs clarification");
            return Ok(ApprovalRoute::NeedsClarification {
                reason: rationale.trim().to_string(),
            });
        }
        other => {
            return Err(CoreError::other(format!(
                "unsupported approval decision: {other}"
            )));
        }
    };

    let decision = UserDecision {
        approval_id: approval.id.clone(),
        kind,
        selected_option_id,
        message,
        edits: serde_json::Value::Null,
    };
    validate_user_decision_shape(&decision)?;
    if let Some(selected_id) = decision.selected_option_id.as_deref() {
        if !approval.options.iter().any(|o| o.id == selected_id) {
            return Err(CoreError::other(format!(
                "approval decision selected unknown option: {selected_id}"
            )));
        }
    }
    Ok(ApprovalRoute::Decision(decision))
}

#[cfg(test)]
pub(super) fn parse_approval_decision_response(
    text: &str,
    approval: &ApprovalRequest,
) -> Result<UserDecision> {
    match parse_approval_route_response(text, approval)? {
        ApprovalRoute::Decision(decision) => Ok(decision),
        ApprovalRoute::NeedsClarification { reason } => Err(CoreError::other(format!(
            "approval decision needs clarification: {reason}"
        ))),
    }
}

pub(super) fn update_build_restrictions_tool_spec() -> AgentToolSpec {
    AgentToolSpec {
        name: UPDATE_BUILD_RESTRICTIONS_TOOL.to_string(),
        description:
            "Validate and apply a full replacement patch for typed modpack build restrictions."
                .to_string(),
        input_schema: update_build_restrictions_input_schema(),
        output_schema: update_build_restrictions_output_schema(),
    }
}

fn update_build_restrictions_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "base_revision": {
                "type": "integer",
                "minimum": 0
            },
            "patch": build_restriction_patch_schema()
        },
        "required": ["base_revision", "patch"]
    })
}

fn build_restriction_patch_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "minecraft_version": {
                "type": ["string", "null"],
                "description": "Explicit Minecraft version such as 1.20.1, or null when unspecified."
            },
            "minecraft_version_requirement": {
                "type": ["string", "null"],
                "description": "Raw version requirement text from the user, such as 1.20.1, 1.20.x, <=1.19.x, or null when unspecified."
            },
            "loader": {
                "type": ["string", "null"],
                "enum": ["fabric", "forge", "neoforge", "quilt", null],
                "description": "Explicit mod loader, or null when unspecified."
            },
            "feature_tags": {
                "type": "array",
                "maxItems": 8,
                "items": {
                    "type": "string"
                }
            },
            "notes": {
                "type": ["string", "null"]
            }
        },
        "required": ["minecraft_version", "minecraft_version_requirement", "loader", "feature_tags", "notes"]
    })
}

fn update_build_restrictions_output_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "restrictions": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "revision": { "type": "integer" },
                    "minecraft_version": { "type": ["string", "null"] },
                    "minecraft_version_requirement": { "type": ["string", "null"] },
                    "loader": {
                        "type": ["string", "null"],
                        "enum": ["fabric", "forge", "neoforge", "quilt", null]
                    },
                    "feature_tags": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "notes": { "type": ["string", "null"] }
                },
                "required": ["revision", "minecraft_version", "minecraft_version_requirement", "loader", "feature_tags", "notes"]
            },
            "missing_fields": {
                "type": "array",
                "items": { "type": "string" }
            },
            "warnings": {
                "type": "array",
                "items": { "type": "string" }
            }
        },
        "required": ["restrictions", "missing_fields", "warnings"]
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(super) struct SearchQueryOutput {
    pub queries: Vec<String>,
}

#[cfg(test)]
pub(super) fn search_queries(model_text: &str) -> Result<Vec<String>> {
    let queries = parse_search_query_response(model_text, "base modpack search")?;
    Ok(dedupe_queries(queries).into_iter().take(6).collect())
}

impl SearchQueryOutput {
    pub(super) fn into_queries(self, context: &str) -> Result<Vec<String>> {
        let text = serde_json::to_string(&self).map_err(|source| CoreError::Parse {
            what: "search query output".into(),
            source,
        })?;
        parse_search_query_response(&text, context)
    }
}

fn parse_search_query_response(model_text: &str, context: &str) -> Result<Vec<String>> {
    let value = parse_first_json_object(model_text).ok_or_else(|| {
        CoreError::other(format!(
            "could not parse {context} query JSON from model output: {model_text}"
        ))
    })?;
    let raw_queries = value
        .get("queries")
        .and_then(|v| v.as_array())
        .ok_or_else(|| CoreError::other(format!("{context} model output missing queries[]")))?;
    let queries = raw_queries
        .iter()
        .filter_map(|v| v.as_str())
        .map(clean_query_text)
        .filter(|s| is_search_query_text(s))
        .take(4)
        .collect::<Vec<_>>();

    if queries.is_empty() {
        return Err(CoreError::other(format!(
            "{context} model output did not contain any usable query"
        )));
    }

    Ok(dedupe_queries(queries))
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(super) struct ModQueryOutput {
    pub queries: Vec<String>,
    pub retain_existing_mods: bool,
    pub remove_existing_mod_ids: Vec<String>,
}

impl ModQueryOutput {
    pub(super) fn into_plan(self) -> Result<GeneratedModSearchPlan> {
        let text = serde_json::to_string(&self).map_err(|source| CoreError::Parse {
            what: "extra mod query output".into(),
            source,
        })?;
        parse_mod_query_response(&text)
    }
}

pub(super) fn parse_mod_query_response(model_text: &str) -> Result<GeneratedModSearchPlan> {
    let value = parse_first_json_object(model_text).ok_or_else(|| {
        CoreError::other(format!(
            "could not parse extra mod search query JSON from model output: {model_text}"
        ))
    })?;
    let raw_queries = value
        .get("queries")
        .and_then(|v| v.as_array())
        .ok_or_else(|| CoreError::other("extra mod search model output missing queries[]"))?;
    let queries = raw_queries
        .iter()
        .filter_map(|v| v.as_str())
        .map(clean_query_text)
        .filter(|s| is_search_query_text(s))
        .take(4)
        .collect::<Vec<_>>();
    let remove_existing_mod_ids = value
        .get("remove_existing_mod_ids")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            CoreError::other("extra mod search model output missing remove_existing_mod_ids[]")
        })?
        .iter()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let retain_existing_mods = value
        .get("retain_existing_mods")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| {
            CoreError::other("extra mod search model output missing retain_existing_mods")
        })?;

    Ok(GeneratedModSearchPlan {
        model: String::new(),
        queries: dedupe_queries(queries),
        retain_existing_mods,
        remove_existing_mod_ids,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum CustomizationCritiqueVerdictOutput {
    Pass,
    Revise,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub(super) struct CustomizationCritiqueOutput {
    pub verdict: CustomizationCritiqueVerdictOutput,
    pub remove_project_ids: Vec<String>,
    pub additional_queries: Vec<String>,
    pub rationale: String,
}

impl CustomizationCritiqueOutput {
    pub(super) fn into_critique(self) -> Result<GeneratedCustomizationCritique> {
        let text = serde_json::to_string(&self).map_err(|source| CoreError::Parse {
            what: "customization critique output".into(),
            source,
        })?;
        parse_customization_critique_response(&text)
    }
}

pub(super) fn parse_customization_critique_response(
    model_text: &str,
) -> Result<GeneratedCustomizationCritique> {
    let value = parse_first_json_object(model_text).ok_or_else(|| {
        CoreError::other(format!(
            "could not parse customization critique JSON from model output: {model_text}"
        ))
    })?;
    let verdict = match value.get("verdict").and_then(|v| v.as_str()) {
        Some("pass") => CustomizationCritiqueVerdict::Pass,
        Some("revise") => CustomizationCritiqueVerdict::Revise,
        Some(other) => {
            return Err(CoreError::other(format!(
                "unsupported customization critique verdict: {other}"
            )));
        }
        None => return Err(CoreError::other("customization critique missing verdict")),
    };
    let remove_project_ids = value
        .get("remove_project_ids")
        .and_then(|v| v.as_array())
        .ok_or_else(|| CoreError::other("customization critique missing remove_project_ids[]"))?
        .iter()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let additional_queries = value
        .get("additional_queries")
        .and_then(|v| v.as_array())
        .ok_or_else(|| CoreError::other("customization critique missing additional_queries[]"))?
        .iter()
        .filter_map(|v| v.as_str())
        .map(clean_query_text)
        .filter(|s| is_search_query_text(s))
        .take(3)
        .collect::<Vec<_>>();
    let rationale = value
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    Ok(GeneratedCustomizationCritique {
        model: String::new(),
        verdict,
        remove_project_ids,
        additional_queries: dedupe_queries(additional_queries),
        rationale,
    })
}

fn clean_query_text(text: &str) -> String {
    let mut trimmed = text.trim().trim_matches('"').trim();
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        trimmed = rest.trim();
    } else if let Some((marker, rest)) = trimmed.split_once(' ') {
        let marker = marker.trim();
        let ordinal = marker
            .strip_suffix('.')
            .or_else(|| marker.strip_suffix(')'))
            .unwrap_or(marker);
        if !ordinal.is_empty() && ordinal.chars().all(|c| c.is_ascii_digit()) {
            trimmed = rest.trim();
        }
    }
    trimmed.trim_matches('"').to_string()
}

fn is_search_query_text(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.ends_with(':') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    !matches!(
        lower.as_str(),
        "queries" | "search queries" | "search query" | "base modpack search"
    )
}

pub(super) fn dedupe_queries(queries: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    queries
        .into_iter()
        .filter(|q| seen.insert(q.to_ascii_lowercase()))
        .collect()
}
