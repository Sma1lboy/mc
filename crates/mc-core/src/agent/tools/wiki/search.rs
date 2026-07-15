use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::WikiChunk;

#[derive(Debug, Clone)]
pub(super) struct SearchQuery {
    terms: Vec<String>,
    special_terms: Vec<String>,
    normalized_phrase: String,
    pub(super) snippet_terms: Vec<String>,
}

impl SearchQuery {
    pub(super) fn parse(input: &str) -> Self {
        let mut terms = search_terms(input);
        let mut special_terms = symbol_tokens(input);
        for special in &special_terms {
            terms.extend(search_terms(special));
        }
        terms.sort();
        terms.dedup();
        special_terms.sort();
        special_terms.dedup();
        let mut snippet_terms = terms.clone();
        snippet_terms.extend(special_terms.iter().cloned());
        snippet_terms.sort();
        snippet_terms.dedup();
        Self {
            terms,
            special_terms,
            normalized_phrase: normalize_search_text(input),
            snippet_terms,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.special_terms.is_empty()
    }
}

pub(super) fn score_chunk(chunk: &WikiChunk, query: &SearchQuery) -> f32 {
    let title = SearchText::new(&chunk.title);
    let source = SearchText::new(&format!("{} {}", chunk.source_label, chunk.location));
    let content = SearchText::new(&chunk.content);

    let mut score = 0.0_f32;
    if query.normalized_phrase.len() >= 4 {
        if title.normalized.contains(&query.normalized_phrase) {
            score += 14.0;
        }
        if content.normalized.contains(&query.normalized_phrase) {
            score += 8.0;
        }
    }

    for special in &query.special_terms {
        let special = special.as_str();
        if title.lower.contains(special) {
            score += 9.0;
        }
        if content.lower.contains(special) {
            score += 7.0;
        }
        if source.lower.contains(special) {
            score += 4.0;
        }
    }

    for term in &query.terms {
        score += title.term_score(term, 5.0, 2.5);
        score += source.term_score(term, 2.0, 1.0);
        score += content.term_score(term, 1.4, 0.9);
    }

    if score <= 0.0 {
        for term in &query.terms {
            score += title.fuzzy_score(term, 3.0);
            score += source.fuzzy_score(term, 1.0);
            score += content.fuzzy_score(term, 0.7);
        }
    }

    if score > 0.0 {
        score *= source_weight(&chunk.source_label);
    }
    score
}

pub(super) fn normalize_filter_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase())
}

pub(super) fn chunk_matches_kind(chunk: &WikiChunk, kind: Option<&str>) -> bool {
    let Some(kind) = kind else {
        return true;
    };
    chunk
        .kind
        .as_deref()
        .map(|chunk_kind| chunk_kind.eq_ignore_ascii_case(kind))
        .unwrap_or(false)
}

pub(super) fn chunk_matches_target(chunk: &WikiChunk, target_id: Option<&str>) -> bool {
    let Some(target_id) = target_id else {
        return true;
    };
    let Some(structured) = chunk.structured.as_ref() else {
        return false;
    };
    structured_target_ids(structured)
        .into_iter()
        .any(|id| id.eq_ignore_ascii_case(target_id))
}

pub(super) fn chunk_matches_ingredient(chunk: &WikiChunk, ingredient_id: Option<&str>) -> bool {
    let Some(ingredient_id) = ingredient_id else {
        return true;
    };
    let Some(structured) = chunk.structured.as_ref() else {
        return false;
    };
    structured_ingredient_ids(structured)
        .into_iter()
        .any(|id| id.eq_ignore_ascii_case(ingredient_id))
}

pub(super) fn structured_filter_score(
    chunk: &WikiChunk,
    target_id: Option<&str>,
    ingredient_id: Option<&str>,
) -> f32 {
    let mut score = 0.0;
    if target_id.is_some() && chunk_matches_target(chunk, target_id) {
        score += 40.0;
    }
    if ingredient_id.is_some() && chunk_matches_ingredient(chunk, ingredient_id) {
        score += 20.0;
    }
    score
}

pub(super) fn source_priority_score(chunk: &WikiChunk) -> f32 {
    match chunk.kind.as_deref() {
        Some("recipe") | Some("recipe_override") => match structured_source_type(&chunk.structured)
        {
            Some("kubejs") => 30.0,
            Some("datapack") => 24.0,
            Some("local") => 18.0,
            Some("mod_jar") => 8.0,
            _ => 0.0,
        },
        _ => 0.0,
    }
}

fn structured_source_type(structured: &Option<Value>) -> Option<&str> {
    structured
        .as_ref()?
        .pointer("/source/type")
        .and_then(|value| value.as_str())
}

fn structured_target_ids(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(id) = value.pointer("/result/id").and_then(|value| value.as_str()) {
        out.push(id.to_string());
    }
    if let Some(id) = value.pointer("/target/id").and_then(|value| value.as_str()) {
        out.push(id.to_string());
    }
    if let Some(id) = value.pointer("/target_id").and_then(|value| value.as_str()) {
        out.push(id.to_string());
    }
    if let Some(id) = value.pointer("/id").and_then(|value| value.as_str()) {
        if value
            .get("kind")
            .and_then(|kind| kind.as_str())
            .map(|kind| matches!(kind, "tag" | "recipe_override"))
            .unwrap_or(false)
        {
            out.push(id.to_string());
        }
    }
    collect_result_ids(value.get("results"), &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect_result_ids(value: Option<&Value>, out: &mut Vec<String>) {
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                collect_result_ids(Some(item), out);
            }
        }
        Some(Value::Object(map)) => {
            for key in ["id", "item"] {
                if let Some(id) = map.get(key).and_then(|value| value.as_str()) {
                    out.push(id.to_string());
                }
            }
            if let Some(item) = map.get("result").or_else(|| map.get("output")) {
                collect_result_ids(Some(item), out);
            }
        }
        Some(Value::String(id)) => out.push(id.to_string()),
        _ => {}
    }
}

fn structured_ingredient_ids(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_structured_ids(value.get("ingredients"), &mut out);
    collect_structured_ids(value.get("grid"), &mut out);
    collect_structured_ids(value.get("input"), &mut out);
    collect_structured_ids(value.get("replacement"), &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect_structured_ids(value: Option<&Value>, out: &mut Vec<String>) {
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                collect_structured_ids(Some(item), out);
            }
        }
        Some(Value::Object(map)) => {
            if let Some(id) = map.get("id").and_then(|value| value.as_str()) {
                out.push(id.to_string());
            }
            if let Some(item) = map.get("item").and_then(|value| value.as_str()) {
                out.push(item.to_string());
            }
            if let Some(tag) = map.get("tag").and_then(|value| value.as_str()) {
                out.push(format!("#{tag}"));
            }
            for value in map.values() {
                collect_structured_ids(Some(value), out);
            }
        }
        Some(Value::String(id)) => out.push(id.to_string()),
        _ => {}
    }
}

#[derive(Debug)]
struct SearchText {
    lower: String,
    normalized: String,
    tokens: Vec<String>,
    counts: HashMap<String, usize>,
}

impl SearchText {
    fn new(text: &str) -> Self {
        let lower = text.to_ascii_lowercase();
        let normalized = normalize_search_text(text);
        let mut tokens = search_terms(text);
        tokens.extend(
            symbol_tokens(text)
                .into_iter()
                .flat_map(|token| search_terms(&token)),
        );
        tokens.sort();
        let mut counts = HashMap::new();
        for token in &tokens {
            *counts.entry(token.clone()).or_insert(0) += 1;
        }
        tokens.dedup();
        Self {
            lower,
            normalized,
            tokens,
            counts,
        }
    }

    fn term_score(&self, term: &str, exact_weight: f32, fuzzy_weight: f32) -> f32 {
        if let Some(count) = self.counts.get(term) {
            return exact_weight * (*count).min(4) as f32;
        }
        self.fuzzy_score(term, fuzzy_weight)
    }

    fn fuzzy_score(&self, term: &str, weight: f32) -> f32 {
        if term.len() < 3 {
            return 0.0;
        }
        let best = self
            .tokens
            .iter()
            .map(|candidate| fuzzy_similarity(term, candidate))
            .fold(0.0_f32, f32::max);
        if best > 0.66 {
            weight * (1.0 + (best - 0.66) * 2.0)
        } else {
            0.0
        }
    }
}

fn source_weight(source_label: &str) -> f32 {
    let lower = source_label.to_ascii_lowercase();
    if lower == "generated:recipe" {
        1.55
    } else if lower == "generated:recipe-override" {
        1.6
    } else if lower == "generated:ftb-quests" {
        1.35
    } else if lower == "generated:patchouli" {
        1.3
    } else if lower == "generated:tag" {
        1.2
    } else if lower == "generated:project-doc" {
        1.05
    } else if lower == "generated:instance-data" {
        0.75
    } else if lower.contains("kubejs") || lower.contains("scripts") {
        1.15
    } else {
        1.0
    }
}

fn search_terms(query: &str) -> Vec<String> {
    let mut terms = query
        .split(|ch: char| !ch.is_alphanumeric())
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| term.len() >= 2)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    terms.sort();
    terms
}

pub(super) fn symbol_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || matches!(ch, ':' | '_' | '-' | '.' | '/') {
            buf.push(ch.to_ascii_lowercase());
        } else {
            push_symbol_token(&mut tokens, &mut buf);
        }
    }
    push_symbol_token(&mut tokens, &mut buf);
    tokens
}

fn push_symbol_token(tokens: &mut Vec<String>, buf: &mut String) {
    if buf.len() >= 3
        && buf
            .chars()
            .any(|ch| matches!(ch, ':' | '_' | '-' | '.' | '/'))
        && buf.chars().any(|ch| ch.is_ascii_alphabetic())
    {
        tokens.push(std::mem::take(buf));
    } else {
        buf.clear();
    }
}

fn normalize_search_text(text: &str) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

fn fuzzy_similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    if a.len() < 3 || b.len() < 3 {
        return 0.0;
    }
    if is_subsequence(a, b) || is_subsequence(b, a) {
        return 0.82;
    }
    let a_set = trigrams(a);
    let b_set = trigrams(b);
    if a_set.is_empty() || b_set.is_empty() {
        return 0.0;
    }
    let intersection = a_set.intersection(&b_set).count() as f32;
    let union = a_set.union(&b_set).count() as f32;
    intersection / union
}

fn trigrams(input: &str) -> HashSet<String> {
    let chars = input.chars().collect::<Vec<_>>();
    if chars.len() < 3 {
        return HashSet::new();
    }
    chars
        .windows(3)
        .map(|w| w.iter().collect::<String>())
        .collect()
}

fn is_subsequence(short: &str, long: &str) -> bool {
    if short.len() > long.len() {
        return false;
    }
    let mut chars = short.chars();
    let mut next = chars.next();
    for ch in long.chars() {
        if Some(ch) == next {
            next = chars.next();
            if next.is_none() {
                return true;
            }
        }
    }
    next.is_none()
}

pub(super) fn snippet_for_terms(content: &str, terms: &[String]) -> String {
    let lower = content.to_ascii_lowercase();
    let start = terms
        .iter()
        .filter_map(|term| lower.find(term))
        .min()
        .unwrap_or(0);
    let prefix_chars = content[..start.min(content.len())].chars().count();
    content
        .chars()
        .skip(prefix_chars.saturating_sub(80))
        .take(260)
        .collect::<String>()
        .trim()
        .replace('\n', " ")
}
