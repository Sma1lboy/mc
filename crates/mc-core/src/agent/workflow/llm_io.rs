use super::*;

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ApprovalDecisionKind {
    Approve,
    Revise,
    Cancel,
    NeedsClarification,
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

    let decision = match decision.as_str() {
        "approve" => ApprovalDecisionKind::Approve,
        "revise" => ApprovalDecisionKind::Revise,
        "cancel" => ApprovalDecisionKind::Cancel,
        "needs_clarification" => ApprovalDecisionKind::NeedsClarification,
        other => {
            return Err(CoreError::other(format!(
                "unsupported approval decision: {other}"
            )));
        }
    };
    let rationale = value
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    approval_route_from_parts(
        decision,
        raw_selected_option_id,
        raw_message,
        rationale,
        approval,
    )
}

fn approval_route_from_parts(
    decision: ApprovalDecisionKind,
    raw_selected_option_id: Option<String>,
    raw_message: Option<String>,
    rationale: String,
    approval: &ApprovalRequest,
) -> Result<ApprovalRoute> {
    let selected_option_id = raw_selected_option_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let message = raw_message
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let (kind, selected_option_id, message) = match decision {
        ApprovalDecisionKind::Approve => (UserDecisionKind::Approve, selected_option_id, None),
        ApprovalDecisionKind::Revise => (UserDecisionKind::Revise, None, message),
        ApprovalDecisionKind::Cancel => (UserDecisionKind::Cancel, None, None),
        ApprovalDecisionKind::NeedsClarification => {
            let reason = if rationale.trim().is_empty() {
                "approval decision needs clarification"
            } else {
                rationale.trim()
            };
            return Ok(ApprovalRoute::NeedsClarification {
                reason: reason.to_string(),
            });
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
        input_schema: serde_json::to_value(schemars::schema_for!(UpdateBuildRestrictionsInput))
            .expect("UpdateBuildRestrictionsInput schema should serialize"),
        output_schema: serde_json::to_value(schemars::schema_for!(UpdateBuildRestrictionsOutput))
            .expect("UpdateBuildRestrictionsOutput schema should serialize"),
    }
}

#[cfg(test)]
pub(super) fn search_queries(model_text: &str) -> Result<Vec<String>> {
    let queries = parse_search_query_response(model_text, "base modpack search")?;
    Ok(dedupe_queries(queries).into_iter().take(6).collect())
}

#[cfg(test)]
pub(super) fn parse_search_query_response(model_text: &str, context: &str) -> Result<Vec<String>> {
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
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    normalize_search_queries(queries, context, 4)
}

#[cfg(test)]
fn normalize_search_queries(
    queries: Vec<String>,
    context: &str,
    limit: usize,
) -> Result<Vec<String>> {
    let queries = queries
        .into_iter()
        .map(|q| clean_query_text(&q))
        .filter(|s| is_search_query_text(s))
        .take(limit)
        .collect::<Vec<_>>();

    if queries.is_empty() {
        return Err(CoreError::other(format!(
            "{context} model output did not contain any usable query"
        )));
    }

    Ok(dedupe_queries(queries))
}

#[cfg(test)]
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
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let remove_existing_mod_ids = value
        .get("remove_existing_mod_ids")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            CoreError::other("extra mod search model output missing remove_existing_mod_ids[]")
        })?
        .iter()
        .filter_map(|v| v.as_str())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let retain_existing_mods = value
        .get("retain_existing_mods")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| {
            CoreError::other("extra mod search model output missing retain_existing_mods")
        })?;

    Ok(normalize_mod_query_output(
        queries,
        retain_existing_mods,
        remove_existing_mod_ids,
    ))
}

#[cfg(test)]
fn normalize_mod_query_output(
    queries: Vec<String>,
    retain_existing_mods: bool,
    remove_existing_mod_ids: Vec<String>,
) -> GeneratedModSearchPlan {
    let queries = queries
        .into_iter()
        .map(|q| clean_query_text(&q))
        .filter(|s| is_search_query_text(s))
        .take(4)
        .collect::<Vec<_>>();
    let remove_existing_mod_ids = remove_existing_mod_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();

    GeneratedModSearchPlan {
        queries: dedupe_queries(queries),
        retain_existing_mods,
        remove_existing_mod_ids,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ModPlanStep {
    #[serde(default)]
    pub selections: Vec<ModSelection>,
    #[serde(default)]
    pub removals: Vec<String>,
    #[serde(default)]
    pub next_queries: Vec<GoalQuery>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ModSelection {
    pub goal_id: String,
    pub project_id: String,
}

/// Typed output of the base-pack coverage analysis step. Given the requested
/// theme goals and the selected base pack's own (enriched) modlist, the model
/// reports which goals the base pack already satisfies so the planner can skip
/// searching for them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct BaseCoverageReport {
    #[serde(default)]
    pub covered_goals: Vec<BaseCoveredGoal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct BaseCoveredGoal {
    pub goal_id: String,
    /// Titles (or project ids) of the base-pack mods that satisfy this goal.
    #[serde(default)]
    pub covering_mods: Vec<String>,
    #[serde(default)]
    pub rationale: String,
}

impl BaseCoverageReport {
    /// Distinct, trimmed goal ids that are both reported covered and still an
    /// open theme goal we asked about. Hallucinated or duplicate ids are
    /// dropped so the model can only ever close goals that actually exist.
    pub(super) fn covered_goal_ids(&self, open_theme_goal_ids: &HashSet<String>) -> Vec<String> {
        let mut seen = HashSet::new();
        self.covered_goals
            .iter()
            .map(|goal| goal.goal_id.trim().to_string())
            .filter(|id| {
                !id.is_empty() && open_theme_goal_ids.contains(id) && seen.insert(id.clone())
            })
            .collect()
    }

    /// Compact, trace-friendly view of the raw model verdict.
    pub(super) fn trace_payload(&self) -> Vec<serde_json::Value> {
        self.covered_goals
            .iter()
            .map(|goal| {
                serde_json::json!({
                    "goal_id": goal.goal_id.trim(),
                    "covering_mods": goal.covering_mods,
                    "rationale": goal.rationale.trim(),
                })
            })
            .collect()
    }
}

pub(super) fn parse_mod_plan_step_response(
    model_text: &str,
    candidate_project_ids: &HashSet<String>,
    goal_ids: &HashSet<String>,
    default_goal_id: Option<&str>,
) -> Result<ModPlanStep> {
    let mut value = parse_first_json_object(model_text).ok_or_else(|| {
        CoreError::other(format!(
            "could not parse mod plan tool arguments from model output: {model_text}"
        ))
    })?;
    normalize_mod_plan_step_json(&mut value, default_goal_id);
    let step = serde_json::from_value::<ModPlanStep>(value)
        .map_err(|err| CoreError::other(format!("invalid mod plan tool arguments: {err}")))?;
    Ok(step.normalized(candidate_project_ids, goal_ids))
}

fn normalize_mod_plan_step_json(value: &mut serde_json::Value, default_goal_id: Option<&str>) {
    let Some(default_goal_id) = default_goal_id
        .map(str::trim)
        .filter(|goal_id| !goal_id.is_empty())
    else {
        return;
    };
    normalize_goal_scoped_items(value, "selections", default_goal_id, "project_id");
    normalize_goal_scoped_items(value, "next_queries", default_goal_id, "query");
}

fn normalize_goal_scoped_items(
    value: &mut serde_json::Value,
    key: &str,
    default_goal_id: &str,
    text_field: &str,
) {
    let Some(items) = value.get_mut(key).and_then(|v| v.as_array_mut()) else {
        return;
    };
    for item in items {
        if let Some(text) = item.as_str().map(str::trim).filter(|text| !text.is_empty()) {
            *item = serde_json::json!({
                "goal_id": default_goal_id,
                text_field: text,
            });
            continue;
        }
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        let has_goal_id = obj
            .get("goal_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .is_some_and(|goal_id| !goal_id.is_empty());
        if !has_goal_id {
            obj.insert(
                "goal_id".to_string(),
                serde_json::Value::String(default_goal_id.to_string()),
            );
        }
    }
}

pub(super) fn parse_base_coverage_response(model_text: &str) -> Result<BaseCoverageReport> {
    let value = parse_first_json_object(model_text).ok_or_else(|| {
        CoreError::other(format!(
            "could not parse base coverage tool arguments from model output: {model_text}"
        ))
    })?;
    serde_json::from_value::<BaseCoverageReport>(value)
        .map_err(|err| CoreError::other(format!("invalid base coverage tool arguments: {err}")))
}

impl ModPlanStep {
    pub(super) fn normalized(
        self,
        candidate_project_ids: &HashSet<String>,
        goal_ids: &HashSet<String>,
    ) -> Self {
        let mut seen_selections = HashSet::new();
        let selections = self
            .selections
            .into_iter()
            .map(|selection| ModSelection {
                goal_id: selection.goal_id.trim().to_string(),
                project_id: selection.project_id.trim().to_string(),
            })
            .filter(|selection| {
                !selection.goal_id.is_empty()
                    && !selection.project_id.is_empty()
                    && goal_ids.contains(&selection.goal_id)
                    && candidate_project_ids.contains(&selection.project_id)
                    && seen_selections
                        .insert((selection.goal_id.clone(), selection.project_id.clone()))
            })
            .collect();

        let mut seen_removals = HashSet::new();
        let removals = self
            .removals
            .into_iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty() && seen_removals.insert(id.clone()))
            .collect();

        let mut seen_queries = HashSet::new();
        let next_queries = self
            .next_queries
            .into_iter()
            .filter_map(|query| {
                let goal_id = query.goal_id.trim().to_string();
                let query_text = clean_mod_query_text(&query.query);
                if goal_id.is_empty()
                    || !goal_ids.contains(&goal_id)
                    || !is_search_query_text(&query_text)
                {
                    return None;
                }
                let key = (goal_id.clone(), query_text.to_ascii_lowercase());
                if !seen_queries.insert(key) {
                    return None;
                }
                Some(GoalQuery {
                    goal_id,
                    query: query_text,
                })
            })
            .take(6)
            .collect();

        Self {
            selections,
            removals,
            next_queries,
            rationale: self.rationale.trim().to_string(),
        }
    }
}

pub(super) fn normalize_mod_search_query(text: &str) -> Option<String> {
    let query = clean_mod_query_text(text);
    if is_search_query_text(&query) {
        Some(query)
    } else {
        None
    }
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

fn clean_mod_query_text(text: &str) -> String {
    let mut query = clean_query_text(text);
    if query.trim().is_empty() {
        return query;
    }

    let lower = query.to_ascii_lowercase();
    if lower.contains("immersive portals") {
        return "Immersive Portals".to_string();
    }

    query = truncate_before_case_insensitive(
        &query,
        &[
            ", if ",
            " if compatible",
            " compatible with ",
            " compatibility with ",
            " compatible for ",
            " compatibility for ",
        ],
    );
    query = strip_known_prefixes(
        &query,
        &[
            "please search again and add ",
            "please search for ",
            "please add ",
            "search again for ",
            "search for ",
            "add ",
            "find ",
            "include ",
        ],
    );
    query = strip_known_suffixes(
        &query,
        &[
            " to the extra mods",
            " to extra mods",
            " extra mods",
            " extra mod",
            " mod set",
            " mods",
            " mod",
        ],
    );

    let tokens = query
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .filter(|token| !is_mod_query_noise_token(token))
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        return String::new();
    }

    let joined = tokens.join(" ");
    let lower_joined = joined.to_ascii_lowercase();
    if lower_joined.contains("quality of life") {
        return "quality of life".to_string();
    }
    if lower_joined.contains("realistic portals") {
        return "realistic portals".to_string();
    }

    tokens.into_iter().take(5).collect::<Vec<_>>().join(" ")
}

fn truncate_before_case_insensitive(text: &str, needles: &[&str]) -> String {
    let lower = text.to_ascii_lowercase();
    let cut = needles
        .iter()
        .filter_map(|needle| lower.find(needle))
        .min()
        .unwrap_or(text.len());
    text[..cut].trim().to_string()
}

fn strip_known_prefixes(text: &str, prefixes: &[&str]) -> String {
    let mut out = text.trim().to_string();
    loop {
        let lower = out.to_ascii_lowercase();
        let Some(prefix) = prefixes
            .iter()
            .find(|prefix| lower.starts_with(**prefix))
            .copied()
        else {
            break;
        };
        out = out[prefix.len()..].trim().to_string();
    }
    out
}

fn strip_known_suffixes(text: &str, suffixes: &[&str]) -> String {
    let mut out = text.trim().trim_end_matches('.').trim().to_string();
    loop {
        let lower = out.to_ascii_lowercase();
        let Some(suffix) = suffixes
            .iter()
            .find(|suffix| lower.ends_with(**suffix))
            .copied()
        else {
            break;
        };
        let keep = out.len().saturating_sub(suffix.len());
        out = out[..keep].trim().trim_end_matches(',').trim().to_string();
    }
    out
}

fn is_mod_query_noise_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    if lower.chars().any(|c| c.is_ascii_digit())
        && lower
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == 'x')
    {
        return true;
    }
    matches!(
        lower.as_str(),
        "fabric"
            | "forge"
            | "quilt"
            | "neoforge"
            | "minecraft"
            | "mc"
            | "modrinth"
            | "compatible"
            | "compatibility"
            | "current"
            | "selected"
            | "base"
            | "pack"
            | "loader"
            | "version"
            | "versions"
            | "with"
            | "for"
            | "to"
            | "the"
            | "and"
            | "if"
            | "it"
            | "is"
    )
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
