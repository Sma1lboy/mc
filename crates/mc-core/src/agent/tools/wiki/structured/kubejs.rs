use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{IoResultExt, Result as CoreResult};

use super::super::sources::{
    read_text_file_bounded, regular_dir, relative_slash_path, WIKI_FILE_MAX_BYTES,
};
use super::super::WikiSourceDocument;
use super::recipes::recipe_document_from_json;

pub(super) fn read_kubejs_recipe_script_documents(
    root: &Path,
    labels: &HashMap<String, String>,
) -> CoreResult<Vec<WikiSourceDocument>> {
    let scripts_dir = root.join("kubejs").join("server_scripts");
    if !regular_dir(&scripts_dir) {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_files_with_extension(&scripts_dir, "js", &mut files)?;
    files.sort();

    let mut docs = Vec::new();
    for file in files {
        let Some(content) = read_text_file_bounded(&file, WIKI_FILE_MAX_BYTES) else {
            continue;
        };
        docs.extend(kubejs_recipe_documents_from_script(
            root, &file, &content, labels,
        ));
    }
    Ok(docs)
}

fn collect_files_with_extension(
    dir: &Path,
    extension: &str,
    files: &mut Vec<PathBuf>,
) -> CoreResult<()> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in read {
        let entry = entry.with_path(dir)?;
        let path = entry.path();
        let kind = entry.file_type().with_path(&path)?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect_files_with_extension(&path, extension, files)?;
        } else if kind.is_file()
            && entry.metadata().with_path(&path)?.len() <= WIKI_FILE_MAX_BYTES
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case(extension))
                .unwrap_or(false)
        {
            files.push(path);
        }
    }
    Ok(())
}

fn kubejs_recipe_documents_from_script(
    root: &Path,
    file: &Path,
    content: &str,
    labels: &HashMap<String, String>,
) -> Vec<WikiSourceDocument> {
    let source_rel = relative_slash_path(root, file);
    let file_uri = source_rel.clone();
    let mut docs = Vec::new();

    for (idx, args) in extract_js_call_arguments(content, "event.remove")
        .into_iter()
        .enumerate()
    {
        if let Some(doc) =
            kubejs_recipe_override_document(&file_uri, &source_rel, idx, "remove", &args, None)
        {
            docs.push(doc);
        }
    }

    for (idx, args) in extract_js_call_arguments(content, "event.replaceInput")
        .into_iter()
        .enumerate()
    {
        if let Some(doc) = kubejs_recipe_override_document(
            &file_uri,
            &source_rel,
            idx,
            "replace_input",
            &args,
            Some("input"),
        ) {
            docs.push(doc);
        }
    }

    for (idx, args) in extract_js_call_arguments(content, "event.replaceOutput")
        .into_iter()
        .enumerate()
    {
        if let Some(doc) = kubejs_recipe_override_document(
            &file_uri,
            &source_rel,
            idx,
            "replace_output",
            &args,
            Some("output"),
        ) {
            docs.push(doc);
        }
    }

    for (idx, args) in extract_js_call_arguments(content, "event.custom")
        .into_iter()
        .enumerate()
    {
        let Some(json) = parse_kubejs_json_like_object(&args) else {
            continue;
        };
        let uri = format!("{file_uri}#custom-{idx}");
        let entry_name = format!("{source_rel}#custom-{idx}");
        if let Some(doc) = recipe_document_from_json(&uri, &source_rel, &entry_name, &json, labels)
        {
            docs.push(doc);
        }
    }

    docs
}

fn kubejs_recipe_override_document(
    file_uri: &str,
    source_rel: &str,
    call_index: usize,
    action: &str,
    args: &str,
    replacement_role: Option<&str>,
) -> Option<WikiSourceDocument> {
    let filter = first_js_object_literal(args).unwrap_or(args);
    let target_id = js_object_string_property(filter, "output")
        .or_else(|| js_object_string_property(filter, "result"));
    let recipe_id = js_object_string_property(filter, "id");
    let input_id = js_object_string_property(filter, "input");
    let quoted_args = js_quoted_strings(args);
    let replacement = replacement_role.and_then(|role| {
        let values = quoted_args
            .iter()
            .filter(|value| {
                Some(value.as_str()) != target_id.as_deref()
                    && Some(value.as_str()) != recipe_id.as_deref()
            })
            .cloned()
            .collect::<Vec<_>>();
        match role {
            "input" if values.len() >= 2 => Some(serde_json::json!({
                "from": ingredient_ref_value(&values[0]),
                "to": ingredient_ref_value(&values[1]),
            })),
            "output" if values.len() >= 2 => Some(serde_json::json!({
                "from": ingredient_ref_value(&values[0]),
                "to": ingredient_ref_value(&values[1]),
            })),
            _ => None,
        }
    });
    if target_id.is_none() && recipe_id.is_none() && input_id.is_none() && replacement.is_none() {
        return None;
    }

    let target = target_id
        .as_deref()
        .map(ingredient_ref_value)
        .unwrap_or(Value::Null);
    let input = input_id
        .as_deref()
        .map(ingredient_ref_value)
        .unwrap_or(Value::Null);
    let structured = serde_json::json!({
        "kind": "recipe_override",
        "action": action,
        "target": target,
        "target_id": target_id,
        "recipe_id": recipe_id,
        "input": input,
        "replacement": replacement,
        "source": {
            "origin": "local",
            "type": "kubejs",
            "uri": file_uri,
            "file": source_rel,
        },
    });
    let mut lines = vec![
        "kind: recipe_override".to_string(),
        format!("action: {action}"),
        format!("source: {source_rel}"),
        format!("entry: kubejs:{source_rel}#{action}-{call_index}"),
    ];
    if let Some(target_id) = target_id {
        lines.push(format!("target: {target_id}"));
    }
    if let Some(recipe_id) = recipe_id {
        lines.push(format!("recipe_id: {recipe_id}"));
    }
    if let Some(input_id) = input_id {
        lines.push(format!("input: {input_id}"));
    }
    if let Some(replacement) = structured
        .get("replacement")
        .filter(|value| !value.is_null())
    {
        lines.push(format!("replacement: {replacement}"));
    }

    let target_title = structured
        .pointer("/target/id")
        .and_then(|value| value.as_str())
        .or_else(|| structured.get("recipe_id").and_then(|value| value.as_str()))
        .unwrap_or("recipe");
    Some(WikiSourceDocument::structured(
        format!("KubeJS recipe override: {action} {target_title}"),
        "generated:recipe-override".to_string(),
        format!("generated://recipe-override/{source_rel}#{action}-{call_index}"),
        lines.join("\n"),
        "recipe_override",
        structured,
    ))
}

fn ingredient_ref_value(id: &str) -> Value {
    if let Some(tag) = id.strip_prefix('#') {
        serde_json::json!({ "kind": "tag", "id": format!("#{tag}"), "label": format!("#{tag}") })
    } else {
        serde_json::json!({ "kind": "item", "id": id, "label": id })
    }
}

fn extract_js_call_arguments(content: &str, marker: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = content[cursor..].find(marker) {
        let start = cursor + relative + marker.len();
        let Some(open_relative) = content[start..].find('(') else {
            break;
        };
        let open = start + open_relative;
        if let Some((close, args)) = balanced_delimited(content, open, '(', ')') {
            out.push(args.to_string());
            cursor = close + 1;
        } else {
            break;
        }
    }
    out
}

fn first_js_object_literal(input: &str) -> Option<&str> {
    let open = input.find('{')?;
    balanced_delimited(input, open, '{', '}').map(|(_, body)| body)
}

fn parse_kubejs_json_like_object(input: &str) -> Option<String> {
    let object = first_js_object_literal(input)?;
    let object = normalize_kubejs_object_body(object)?;
    let json = format!("{{{object}}}");
    serde_json::from_str::<Value>(&json).ok()?;
    Some(json)
}

fn normalize_kubejs_object_body(input: &str) -> Option<String> {
    let chars = input.chars().collect::<Vec<_>>();
    let mut out = String::new();
    let mut idx = 0usize;
    while idx < chars.len() {
        let ch = chars[idx];
        if ch == '\'' || ch == '"' {
            let (value, next) = read_js_string_chars(&chars, idx)?;
            out.push_str(&serde_json::to_string(&value).ok()?);
            idx = next;
            continue;
        }
        if ch == '/' && chars.get(idx + 1) == Some(&'/') {
            idx += 2;
            while idx < chars.len() && chars[idx] != '\n' {
                idx += 1;
            }
            continue;
        }
        if ch == '/' && chars.get(idx + 1) == Some(&'*') {
            idx += 2;
            while idx + 1 < chars.len() && !(chars[idx] == '*' && chars[idx + 1] == '/') {
                idx += 1;
            }
            idx = (idx + 2).min(chars.len());
            continue;
        }
        if is_js_identifier_start(ch) {
            let start = idx;
            idx += 1;
            while idx < chars.len() && is_js_identifier_part(chars[idx]) {
                idx += 1;
            }
            let ident = chars[start..idx].iter().collect::<String>();
            let mut lookahead = idx;
            while lookahead < chars.len() && chars[lookahead].is_whitespace() {
                lookahead += 1;
            }
            if chars.get(lookahead) == Some(&':') {
                out.push_str(&serde_json::to_string(&ident).ok()?);
            } else {
                out.push_str(&ident);
            }
            continue;
        }
        out.push(ch);
        idx += 1;
    }
    Some(remove_trailing_json_commas(&out))
}

fn read_js_string_chars(chars: &[char], start: usize) -> Option<(String, usize)> {
    let quote = *chars.get(start)?;
    let mut idx = start + 1;
    let mut out = String::new();
    while idx < chars.len() {
        let ch = chars[idx];
        if ch == quote {
            return Some((out, idx + 1));
        }
        if ch != '\\' {
            out.push(ch);
            idx += 1;
            continue;
        }
        idx += 1;
        let escaped = *chars.get(idx)?;
        match escaped {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'b' => out.push('\u{0008}'),
            'f' => out.push('\u{000c}'),
            '\\' | '"' | '\'' | '/' => out.push(escaped),
            'u' if idx + 4 < chars.len() => {
                let code = chars[idx + 1..=idx + 4].iter().collect::<String>();
                let parsed = u32::from_str_radix(&code, 16).ok()?;
                out.push(char::from_u32(parsed)?);
                idx += 4;
            }
            other => out.push(other),
        }
        idx += 1;
    }
    None
}

fn is_js_identifier_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphabetic()
}

fn is_js_identifier_part(ch: char) -> bool {
    is_js_identifier_start(ch) || ch.is_ascii_digit()
}

fn remove_trailing_json_commas(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut out = String::new();
    let mut idx = 0usize;
    let mut string_quote: Option<char> = None;
    let mut escaped = false;
    while idx < chars.len() {
        let ch = chars[idx];
        if let Some(quote) = string_quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                string_quote = None;
            }
            idx += 1;
            continue;
        }
        if ch == '"' {
            string_quote = Some(ch);
            out.push(ch);
            idx += 1;
            continue;
        }
        if ch == ',' {
            let mut lookahead = idx + 1;
            while lookahead < chars.len() && chars[lookahead].is_whitespace() {
                lookahead += 1;
            }
            if matches!(chars.get(lookahead), Some(&'}') | Some(&']')) {
                idx += 1;
                continue;
            }
        }
        out.push(ch);
        idx += 1;
    }
    out
}

fn js_object_string_property(object: &str, key: &str) -> Option<String> {
    for pattern in [
        format!("{key}:"),
        format!("\"{key}\":"),
        format!("'{key}':"),
    ] {
        if let Some(idx) = object.find(&pattern) {
            let value_start = idx + pattern.len();
            return read_js_string_or_bare_value(&object[value_start..]);
        }
    }
    None
}

fn read_js_string_or_bare_value(input: &str) -> Option<String> {
    let input = input.trim_start();
    let first = input.chars().next()?;
    if first == '\'' || first == '"' {
        let quote = first;
        let mut escaped = false;
        let mut out = String::new();
        for ch in input[first.len_utf8()..].chars() {
            if escaped {
                out.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                return Some(out);
            }
            out.push(ch);
        }
        return None;
    }
    let value = input
        .split(|ch: char| ch == ',' || ch == '}' || ch == ')' || ch.is_whitespace())
        .next()?
        .trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn js_quoted_strings(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = input.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if ch != '\'' && ch != '"' {
            continue;
        }
        let quote = ch;
        let mut escaped = false;
        let mut value = String::new();
        for (end_idx, next) in chars.by_ref() {
            if escaped {
                value.push(next);
                escaped = false;
                continue;
            }
            if next == '\\' {
                escaped = true;
                continue;
            }
            if next == quote {
                out.push(value);
                break;
            }
            value.push(next);
            if end_idx <= idx {
                break;
            }
        }
    }
    out
}

fn balanced_delimited(
    input: &str,
    open_idx: usize,
    open: char,
    close: char,
) -> Option<(usize, &str)> {
    let mut depth = 0usize;
    let mut string_quote: Option<char> = None;
    let mut escaped = false;
    let body_start = open_idx + open.len_utf8();
    for (idx, ch) in input[open_idx..].char_indices() {
        let idx = open_idx + idx;
        if let Some(quote) = string_quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote {
                string_quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            string_quote = Some(ch);
            continue;
        }
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some((idx, &input[body_start..idx]));
            }
        }
    }
    None
}
