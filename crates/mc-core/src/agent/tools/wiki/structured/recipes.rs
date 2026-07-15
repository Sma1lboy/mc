use std::collections::HashMap;

use serde_json::Value;

use super::super::chunk::stable_hex;
use super::super::WikiSourceDocument;

const TAG_STRUCTURED_VALUES_MAX: usize = 128;

pub(super) fn labels_from_lang_json(content: &str) -> HashMap<String, String> {
    let Ok(value) = serde_json::from_str::<HashMap<String, String>>(content) else {
        return HashMap::new();
    };
    value
        .into_iter()
        .filter_map(|(key, label)| translation_key_item_id(&key).map(|id| (id, label)))
        .collect()
}

fn translation_key_item_id(key: &str) -> Option<String> {
    let rest = key
        .strip_prefix("item.")
        .or_else(|| key.strip_prefix("block."))?;
    let (namespace, path) = rest.split_once('.')?;
    if namespace.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{namespace}:{}", path.replace('.', "_")))
}

pub(super) fn recipe_document_from_json(
    uri: &str,
    source_rel: &str,
    entry_name: &str,
    content: &str,
    labels: &HashMap<String, String>,
) -> Option<WikiSourceDocument> {
    let value: Value = serde_json::from_str(content).ok()?;
    let result = recipe_result_value(&value, labels)?;
    let result_id = result.get("id")?.as_str()?.to_string();
    let result_label = result
        .get("label")
        .and_then(|value| value.as_str())
        .unwrap_or(&result_id)
        .to_string();
    let recipe_type = value
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string();
    let ingredients = recipe_ingredients_value(&value, labels);
    let pattern = recipe_pattern_value(&value);
    let grid = recipe_grid_value(&value, labels);
    let recipe_id = format!("recipe:{}:{}", result_id, stable_hex(uri));
    let structured = serde_json::json!({
        "kind": "recipe",
        "id": recipe_id,
        "type": recipe_type,
        "result": result,
        "ingredients": ingredients,
        "pattern": pattern,
        "grid": grid,
        "source": {
            "origin": "local",
            "type": source_type_for_uri(uri),
            "uri": uri,
            "file": source_rel,
        },
    });
    let mut lines = vec![
        "kind: recipe".to_string(),
        format!("title: {result_label}"),
        format!("result: {result_id}"),
        format!("result_label: {result_label}"),
        format!("recipe_type: {recipe_type}"),
        format!("source: {source_rel}"),
        format!("entry: {entry_name}"),
    ];
    for ingredient in collect_ingredient_terms(structured.get("ingredients")) {
        lines.push(format!("ingredient: {ingredient}"));
    }
    if let Some(rows) = pattern.as_array() {
        for row in rows.iter().filter_map(|row| row.as_str()) {
            lines.push(format!("pattern: {row}"));
        }
    }
    Some(WikiSourceDocument::structured(
        format!("Recipe: {result_label} ({result_id})"),
        "generated:recipe".to_string(),
        format!("generated://recipe/{entry_name}#{}", stable_hex(uri)),
        lines.join("\n"),
        "recipe",
        structured,
    ))
}

fn recipe_result_value(value: &Value, labels: &HashMap<String, String>) -> Option<Value> {
    let result = primary_recipe_result(value)?;
    let (id, count) = if let Some(id) = result.as_str() {
        (id.to_string(), 1u64)
    } else {
        let object = result.as_object()?;
        let id = object
            .get("item")
            .or_else(|| object.get("id"))
            .and_then(|value| value.as_str())?
            .to_string();
        let count = object
            .get("count")
            .and_then(|value| value.as_u64())
            .unwrap_or(1);
        (id, count)
    };
    Some(serde_json::json!({
        "id": id,
        "label": item_label(&id, labels),
        "count": count,
    }))
}

fn primary_recipe_result(value: &Value) -> Option<&Value> {
    if let Some(result) = value.get("result").or_else(|| value.get("output")) {
        return Some(result);
    }
    match value.get("results") {
        Some(Value::Array(items)) => items.iter().find(|item| recipe_result_id(item).is_some()),
        Some(value) if recipe_result_id(value).is_some() => Some(value),
        _ => None,
    }
}

fn recipe_result_id(value: &Value) -> Option<&str> {
    if let Some(id) = value.as_str() {
        return Some(id);
    }
    value
        .as_object()?
        .get("item")
        .or_else(|| value.get("id"))
        .and_then(|value| value.as_str())
}

fn recipe_ingredients_value(value: &Value, labels: &HashMap<String, String>) -> Value {
    if let Some(key) = value.get("key").and_then(|value| value.as_object()) {
        let mut out = serde_json::Map::new();
        for (symbol, ingredient) in key {
            out.insert(symbol.clone(), ingredient_value(ingredient, labels));
        }
        return Value::Object(out);
    }
    if let Some(items) = value.get("ingredients").and_then(|value| value.as_array()) {
        return Value::Array(
            items
                .iter()
                .map(|ingredient| ingredient_value(ingredient, labels))
                .collect(),
        );
    }
    Value::Null
}

fn recipe_pattern_value(value: &Value) -> Value {
    value
        .get("pattern")
        .and_then(|value| value.as_array())
        .map(|rows| {
            Value::Array(
                rows.iter()
                    .filter_map(|row| row.as_str())
                    .map(|row| Value::String(row.to_string()))
                    .collect(),
            )
        })
        .unwrap_or(Value::Null)
}

fn recipe_grid_value(value: &Value, labels: &HashMap<String, String>) -> Value {
    let Some(pattern) = value.get("pattern").and_then(|value| value.as_array()) else {
        return Value::Null;
    };
    let Some(key) = value.get("key").and_then(|value| value.as_object()) else {
        return Value::Null;
    };
    let rows = pattern
        .iter()
        .filter_map(|row| row.as_str())
        .map(|row| {
            Value::Array(
                row.chars()
                    .map(|symbol| {
                        if symbol == ' ' {
                            Value::Null
                        } else {
                            key.get(&symbol.to_string())
                                .map(|ingredient| ingredient_value(ingredient, labels))
                                .unwrap_or(Value::Null)
                        }
                    })
                    .collect(),
            )
        })
        .collect();
    Value::Array(rows)
}

fn ingredient_value(value: &Value, labels: &HashMap<String, String>) -> Value {
    if let Some(id) = value.as_str() {
        return serde_json::json!({
            "kind": "item",
            "id": id,
            "label": item_label(id, labels),
        });
    }
    if let Some(array) = value.as_array() {
        return serde_json::json!({
            "kind": "alternatives",
            "options": array
                .iter()
                .map(|item| ingredient_value(item, labels))
                .collect::<Vec<_>>(),
        });
    }
    let Some(object) = value.as_object() else {
        return serde_json::json!({ "kind": "unknown", "raw": value });
    };
    if let Some(id) = object
        .get("item")
        .or_else(|| object.get("id"))
        .and_then(|value| value.as_str())
    {
        return serde_json::json!({
            "kind": "item",
            "id": id,
            "label": item_label(id, labels),
        });
    }
    if let Some(tag) = object.get("tag").and_then(|value| value.as_str()) {
        return serde_json::json!({
            "kind": "tag",
            "id": format!("#{tag}"),
            "label": format!("#{tag}"),
        });
    }
    serde_json::json!({ "kind": "unknown", "raw": value })
}

fn collect_ingredient_terms(value: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(value) = value {
        collect_ingredient_terms_inner(value, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn collect_ingredient_terms_inner(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_ingredient_terms_inner(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(id) = map.get("id").and_then(|value| value.as_str()) {
                out.push(id.to_string());
            }
            if let Some(label) = map.get("label").and_then(|value| value.as_str()) {
                out.push(label.to_string());
            }
            for value in map.values() {
                collect_ingredient_terms_inner(value, out);
            }
        }
        _ => {}
    }
}

pub(super) fn tag_document_from_json(
    uri: &str,
    source_rel: &str,
    entry_name: &str,
    content: &str,
    labels: &HashMap<String, String>,
) -> Option<WikiSourceDocument> {
    let value: Value = serde_json::from_str(content).ok()?;
    let tag_id = tag_id_from_entry_name(entry_name)?;
    let values = value
        .get("values")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let normalized_values = values
        .into_iter()
        .filter_map(|value| tag_value(value, labels))
        .collect::<Vec<_>>();
    let value_terms = collect_ingredient_terms(Some(&Value::Array(normalized_values.clone())));
    let value_count = normalized_values.len();
    let values_truncated = value_count > TAG_STRUCTURED_VALUES_MAX;
    let structured_values = normalized_values
        .iter()
        .take(TAG_STRUCTURED_VALUES_MAX)
        .cloned()
        .collect::<Vec<_>>();
    let structured = serde_json::json!({
        "kind": "tag",
        "id": tag_id,
        "replace": value.get("replace").and_then(|value| value.as_bool()).unwrap_or(false),
        "value_count": value_count,
        "values_truncated": values_truncated,
        "values": structured_values,
        "source": {
            "origin": "local",
            "type": source_type_for_uri(uri),
            "uri": uri,
            "file": source_rel,
        },
    });
    let mut lines = vec![
        "kind: tag".to_string(),
        format!("title: {tag_id}"),
        format!("tag: {tag_id}"),
        format!("source: {source_rel}"),
        format!("entry: {entry_name}"),
        format!("value_count: {value_count}"),
    ];
    for value in value_terms {
        lines.push(format!("value: {value}"));
    }
    Some(WikiSourceDocument::structured(
        format!("Tag: {tag_id}"),
        "generated:tag".to_string(),
        format!("generated://tag/{entry_name}#{}", stable_hex(uri)),
        lines.join("\n"),
        "tag",
        structured,
    ))
}

fn tag_value(value: Value, labels: &HashMap<String, String>) -> Option<Value> {
    if let Some(id) = value.as_str() {
        if let Some(tag) = id.strip_prefix('#') {
            return Some(serde_json::json!({
                "kind": "tag",
                "id": format!("#{tag}"),
                "label": format!("#{tag}"),
            }));
        }
        return Some(serde_json::json!({
            "kind": "item",
            "id": id,
            "label": item_label(id.trim_start_matches('#'), labels),
        }));
    }
    let object = value.as_object()?;
    let id = object.get("id").and_then(|value| value.as_str())?;
    if let Some(tag) = id.strip_prefix('#') {
        return Some(serde_json::json!({
            "kind": "tag",
            "id": format!("#{tag}"),
            "required": object.get("required").and_then(|value| value.as_bool()).unwrap_or(true),
            "label": format!("#{tag}"),
        }));
    }
    Some(serde_json::json!({
        "kind": "item",
        "id": id,
        "required": object.get("required").and_then(|value| value.as_bool()).unwrap_or(true),
        "label": item_label(id.trim_start_matches('#'), labels),
    }))
}

pub(super) fn patchouli_document_from_json(
    uri: &str,
    source_rel: &str,
    entry_name: &str,
    content: &str,
) -> Option<WikiSourceDocument> {
    let value: Value = serde_json::from_str(content).ok()?;
    let title = value
        .get("name")
        .or_else(|| value.get("title"))
        .and_then(|value| value.as_str())
        .unwrap_or(entry_name)
        .to_string();
    let mut text_lines = Vec::new();
    collect_patchouli_text(&value, &mut text_lines);
    let structured = serde_json::json!({
        "kind": "patchouli_page",
        "title": title,
        "text": text_lines,
        "source": {
            "origin": "local",
            "type": source_type_for_uri(uri),
            "uri": uri,
            "file": source_rel,
        },
    });
    let mut lines = vec![
        "kind: patchouli_page".to_string(),
        format!("title: {title}"),
        format!("source: {source_rel}"),
        format!("entry: {entry_name}"),
    ];
    lines.extend(
        text_lines
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| format!("text: {line}")),
    );
    Some(WikiSourceDocument::structured(
        format!("Patchouli: {title}"),
        "generated:patchouli".to_string(),
        format!("generated://patchouli/{entry_name}#{}", stable_hex(uri)),
        lines.join("\n"),
        "patchouli_page",
        structured,
    ))
}

fn collect_patchouli_text(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if !text.trim().is_empty() {
                out.push(text.clone());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_patchouli_text(item, out);
            }
        }
        Value::Object(map) => {
            for key in ["name", "title", "text"] {
                if let Some(value) = map.get(key) {
                    collect_patchouli_text(value, out);
                }
            }
            if let Some(pages) = map.get("pages") {
                collect_patchouli_text(pages, out);
            }
        }
        _ => {}
    }
}

pub(super) fn is_lang_entry(name: &str) -> bool {
    let parts = name.split('/').collect::<Vec<_>>();
    parts.len() == 4 && parts[0] == "assets" && parts[2] == "lang" && parts[3].ends_with(".json")
}

pub(super) fn is_recipe_entry(name: &str) -> bool {
    let parts = name.split('/').collect::<Vec<_>>();
    parts.len() >= 4
        && parts[0] == "data"
        && matches!(parts[2], "recipe" | "recipes")
        && parts
            .last()
            .map(|part| part.ends_with(".json"))
            .unwrap_or(false)
}

pub(super) fn is_tag_entry(name: &str) -> bool {
    let parts = name.split('/').collect::<Vec<_>>();
    parts.len() >= 5
        && parts[0] == "data"
        && parts[2] == "tags"
        && parts
            .last()
            .map(|part| part.ends_with(".json"))
            .unwrap_or(false)
}

pub(super) fn is_patchouli_entry(name: &str) -> bool {
    let parts = name.split('/').collect::<Vec<_>>();
    parts.len() >= 5
        && parts[0] == "data"
        && parts[2] == "patchouli_books"
        && parts
            .last()
            .map(|part| part.ends_with(".json"))
            .unwrap_or(false)
}

fn tag_id_from_entry_name(entry_name: &str) -> Option<String> {
    let parts = entry_name.split('/').collect::<Vec<_>>();
    if parts.len() < 5 || parts[0] != "data" || parts[2] != "tags" {
        return None;
    }
    let namespace = parts[1];
    let path = parts[4..].join("/");
    let path = path.strip_suffix(".json").unwrap_or(&path);
    Some(format!("#{namespace}:{path}"))
}

fn source_type_for_uri(uri: &str) -> &'static str {
    if uri.contains(".jar!") {
        "mod_jar"
    } else if uri.contains("/kubejs/") || uri.starts_with("kubejs/") {
        "kubejs"
    } else if uri.contains("/datapacks/") || uri.starts_with("datapacks/") {
        "datapack"
    } else {
        "local"
    }
}

fn item_label(id: &str, labels: &HashMap<String, String>) -> String {
    labels
        .get(id)
        .cloned()
        .unwrap_or_else(|| pretty_item_id(id))
}

fn pretty_item_id(id: &str) -> String {
    let path = id
        .trim_start_matches('#')
        .split_once(':')
        .map(|(_, path)| path)
        .unwrap_or(id);
    path.split(['_', '/', '.'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
