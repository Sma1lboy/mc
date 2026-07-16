use std::path::{Path, PathBuf};

use super::chunk::stable_hex;
use super::{WikiChunk, WikiSensitivity, WikiSourceDocument};

const LOCAL_PATH_REDACTION: &str = "[LOCAL_PATH]";

pub(crate) fn sanitize_private_text(content: &str) -> String {
    let mut sanitized = redact_sensitive_key_lines(content);
    redact_inline_credentials(&mut sanitized);
    redact_local_paths(&mut sanitized);
    sanitized
}

pub(super) fn sanitize_wiki_text(path: &Path, content: &str) -> Option<String> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if extension == "json" {
        let mut value: serde_json::Value = serde_json::from_str(content).ok()?;
        redact_sensitive_json(&mut value);
        return serde_json::to_string_pretty(&value).ok();
    }
    Some(redact_sensitive_key_lines(content))
}

enum SensitiveContinuation {
    Delimiter(&'static str),
    EscapedLine,
    Indented(usize),
}

fn redact_sensitive_key_lines(content: &str) -> String {
    let mut output = Vec::new();
    let mut continuation = None;
    for line in content.lines() {
        if let Some(active) = continuation.as_ref() {
            match active {
                SensitiveContinuation::Delimiter(delimiter) => {
                    if contains_unescaped_delimiter(line, delimiter) {
                        continuation = None;
                    }
                    continue;
                }
                SensitiveContinuation::EscapedLine => {
                    if !ends_with_unescaped_backslash(line) {
                        continuation = None;
                    }
                    continue;
                }
                SensitiveContinuation::Indented(indent) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let line_indent = leading_whitespace(line);
                    if line_indent > *indent {
                        continue;
                    }
                    if line_indent == *indent && is_yaml_sequence_item(line) {
                        continue;
                    }
                    if line_indent == *indent && !looks_like_assignment_or_closing(line) {
                        continuation = None;
                        continue;
                    }
                    continuation = None;
                }
            }
        }

        let Some(delimiter) = sensitive_assignment_delimiter(line) else {
            output.push(line.to_string());
            continue;
        };
        let value = &line[delimiter + 1..];
        continuation = sensitive_continuation(value, leading_whitespace(line));
        output.push(format!("{} [REDACTED]", &line[..=delimiter]));
    }
    output.join("\n")
}

fn looks_like_assignment_or_closing(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with(['}', ']']) {
        return true;
    }
    if matches!(trimmed.as_bytes().first(), Some(b'"' | b'\'')) {
        return quoted_key_delimiter(trimmed);
    }
    let Some((index, delimiter)) = trimmed
        .char_indices()
        .find(|(_, ch)| matches!(ch, ':' | '='))
    else {
        return false;
    };
    let key = trimmed[..index].trim();
    if key.is_empty() {
        return false;
    }
    delimiter == '='
        || trimmed[index + delimiter.len_utf8()..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
}

fn is_yaml_sequence_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed == "-"
        || trimmed
            .strip_prefix('-')
            .and_then(|rest| rest.chars().next())
            .is_some_and(char::is_whitespace)
}

fn quoted_key_delimiter(value: &str) -> bool {
    let bytes = value.as_bytes();
    let quote = bytes[0];
    for index in 1..bytes.len() {
        if bytes[index] != quote {
            continue;
        }
        let escaped = bytes[..index]
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count()
            % 2
            == 1;
        if escaped {
            continue;
        }
        return value[index + 1..]
            .trim_start()
            .as_bytes()
            .first()
            .is_some_and(|byte| matches!(*byte, b':' | b'='));
    }
    false
}

fn redact_inline_credentials(value: &mut String) -> bool {
    let bytes = value.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if is_credential_boundary(bytes, index)
            && starts_ascii_case_insensitive(&bytes[index..], b"bearer")
            && bytes.get(index + 6).is_some_and(u8::is_ascii_whitespace)
        {
            let mut start = index + 6;
            while bytes.get(start).is_some_and(u8::is_ascii_whitespace) {
                start += 1;
            }
            let end = scan_credential_token(bytes, start);
            if end > start {
                spans.push((start, end));
                index = end;
                continue;
            }
        }
        let mut matched = false;
        for prefix in [
            b"sk-".as_slice(),
            b"pk-",
            b"sk_",
            b"pk_",
            b"ghp-",
            b"glpat-",
        ] {
            if is_credential_boundary(bytes, index)
                && starts_ascii_case_insensitive(&bytes[index..], prefix)
            {
                let end = scan_credential_token(bytes, index + prefix.len());
                if end.saturating_sub(index + prefix.len()) >= 12 {
                    spans.push((index, end));
                    index = end;
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            index += 1;
        }
    }
    if spans.is_empty() {
        return false;
    }
    let mut output = Vec::with_capacity(bytes.len());
    let mut copied_until = 0usize;
    for (start, end) in spans {
        output.extend_from_slice(&bytes[copied_until..start]);
        output.extend_from_slice(b"[REDACTED]");
        copied_until = end;
    }
    output.extend_from_slice(&bytes[copied_until..]);
    *value = String::from_utf8(output).expect("credential redaction preserves UTF-8");
    true
}

fn is_credential_boundary(bytes: &[u8], index: usize) -> bool {
    index == 0 || !bytes[index - 1].is_ascii_alphanumeric()
}

fn scan_credential_token(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while bytes.get(end).is_some_and(|byte| {
        matches!(
            byte,
            b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'.'
                | b'_'
                | b'~'
                | b'+'
                | b'/'
                | b'-'
                | b'='
        )
    }) {
        end += 1;
    }
    end
}

fn sensitive_assignment_delimiter(line: &str) -> Option<usize> {
    let mut segment_start = 0usize;
    for (index, ch) in line.char_indices() {
        match ch {
            '=' | ':' => {
                if is_sensitive_key(&line[segment_start..index]) {
                    return Some(index);
                }
                segment_start = index + ch.len_utf8();
            }
            ',' | ';' | '{' | '[' => segment_start = index + ch.len_utf8(),
            _ => {}
        }
    }
    None
}

fn sensitive_continuation(value: &str, indent: usize) -> Option<SensitiveContinuation> {
    let value = value.trim_start();
    if value.is_empty() {
        return Some(SensitiveContinuation::Indented(indent));
    }
    for delimiter in ["\"\"\"", "'''", "`"] {
        if let Some(rest) = value.strip_prefix(delimiter) {
            if !contains_unescaped_delimiter(rest, delimiter) {
                return Some(SensitiveContinuation::Delimiter(delimiter));
            }
        }
    }
    for delimiter in ["\"", "'"] {
        if let Some(rest) = value.strip_prefix(delimiter) {
            if !contains_unescaped_delimiter(rest, delimiter) {
                return Some(SensitiveContinuation::Delimiter(delimiter));
            }
        }
    }
    if ends_with_unescaped_backslash(value) {
        return Some(SensitiveContinuation::EscapedLine);
    }
    let token = value.split_whitespace().next().unwrap_or_default();
    if token
        .strip_prefix(['|', '>'])
        .is_some_and(|rest| rest.chars().all(|ch| matches!(ch, '+' | '-' | '0'..='9')))
    {
        return Some(SensitiveContinuation::Indented(indent));
    }
    None
}

fn contains_unescaped_delimiter(value: &str, delimiter: &str) -> bool {
    value.match_indices(delimiter).any(|(index, _)| {
        value.as_bytes()[..index]
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count()
            % 2
            == 0
    })
}

fn ends_with_unescaped_backslash(value: &str) -> bool {
    value
        .trim_end()
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn leading_whitespace(value: &str) -> usize {
    value.bytes().take_while(u8::is_ascii_whitespace).count()
}

fn redact_sensitive_json(value: &mut serde_json::Value) -> bool {
    let mut redacted = false;
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                redacted |= redact_sensitive_json(item);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if is_sensitive_key(key) {
                    *value = serde_json::Value::String("[REDACTED]".to_string());
                    redacted = true;
                } else {
                    redacted |= redact_sensitive_json(value);
                }
            }
        }
        _ => {}
    }
    redacted
}

fn is_sensitive_key(key: &str) -> bool {
    let mut key = key.trim();
    for prefix in [
        "export const ",
        "export let ",
        "export var ",
        "const ",
        "let ",
        "var ",
    ] {
        if let Some(candidate) = key.strip_prefix(prefix) {
            key = candidate;
            break;
        }
    }
    let key = key.rsplit('.').next().unwrap_or(key).trim();
    let compact_key = !key.chars().any(char::is_whitespace);
    let normalized = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    let ambiguous = [
        "secret",
        "token",
        "auth",
        "credential",
        "credentials",
        "cookie",
        "sessionid",
    ];
    if ambiguous.contains(&normalized.as_str())
        || (compact_key && ambiguous.iter().any(|needle| normalized.ends_with(needle)))
    {
        return true;
    }
    [
        "password",
        "passwd",
        "passphrase",
        "apikey",
        "apitoken",
        "accesstoken",
        "refreshtoken",
        "clientsecret",
        "secretkey",
        "accesskey",
        "privatekey",
        "authorization",
        "authtoken",
        "sessioncookie",
        "authcookie",
        "connectionstring",
        "databaseurl",
        "webhook",
        "webhookurl",
    ]
    .iter()
    .any(|needle| normalized == *needle || normalized.ends_with(needle))
}

fn sanitize_model_text(value: &mut String, roots: &[String]) -> bool {
    let original = value.clone();
    replace_model_roots(value, roots);
    *value = sanitize_private_text(value);
    *value != original
}

fn sanitize_model_identity(value: &mut String, roots: &[String]) -> bool {
    let original = value.clone();
    replace_model_roots(value, roots);
    let before_private = value.clone();
    *value = sanitize_private_text(value);
    let privacy_redacted = *value != before_private;
    sanitize_model_location(value, roots);
    if privacy_redacted {
        value.push('#');
        value.push_str(&stable_hex(&original));
    }
    *value != original
}

pub(super) fn finalize_wiki_documents(docs: &mut [WikiSourceDocument], roots: &[PathBuf]) {
    let root_replacements = model_root_replacements(roots);
    for doc in docs {
        let sanitized_content = sanitize_private_text(&doc.content);
        let mut redacted = sanitized_content != doc.content || doc.content.contains("[REDACTED]");
        doc.content = sanitized_content;

        redacted |= sanitize_model_text(&mut doc.title, &root_replacements);
        redacted |= sanitize_model_text(&mut doc.source_label, &root_replacements);
        redacted |= replace_model_roots(&mut doc.content, &root_replacements);

        redacted |= sanitize_model_identity(&mut doc.uri, &root_replacements);
        redacted |= sanitize_model_identity(&mut doc.provenance.uri, &root_replacements);
        if let Some(structured) = doc.structured.as_mut() {
            redacted |= redact_sensitive_json(structured);
            if let Some(serde_json::Value::String(uri)) = structured.pointer_mut("/source/uri") {
                redacted |= sanitize_model_identity(uri, &root_replacements);
            }
            redacted |= redact_paths_in_json(structured, &root_replacements);
        }
        if redacted {
            doc.provenance.sensitivity = WikiSensitivity::Redacted;
        }
    }
}

pub(super) fn finalize_wiki_chunks(chunks: &mut [WikiChunk], roots: &[PathBuf]) -> bool {
    let root_replacements = model_root_replacements(roots);
    let mut mutated = false;
    for chunk in chunks {
        let sanitized_content = sanitize_private_text(&chunk.content);
        let mut changed = sanitized_content != chunk.content;
        chunk.content = sanitized_content;
        changed |= sanitize_model_text(&mut chunk.title, &root_replacements);
        changed |= sanitize_model_text(&mut chunk.source_label, &root_replacements);
        changed |= sanitize_model_text(&mut chunk.location, &root_replacements);
        changed |= replace_model_roots(&mut chunk.content, &root_replacements);
        changed |= sanitize_model_identity(&mut chunk.provenance.uri, &root_replacements);
        if let Some(structured) = chunk.structured.as_mut() {
            let original = structured.clone();
            redact_sensitive_json(structured);
            if let Some(serde_json::Value::String(uri)) = structured.pointer_mut("/source/uri") {
                sanitize_model_identity(uri, &root_replacements);
            }
            redact_paths_in_json(structured, &root_replacements);
            changed |= *structured != original;
        }
        if (changed || chunk.content.contains("[REDACTED]"))
            && chunk.provenance.sensitivity != WikiSensitivity::Redacted
        {
            chunk.provenance.sensitivity = WikiSensitivity::Redacted;
            changed = true;
        }
        mutated |= changed;
    }
    mutated
}

fn model_root_replacements(roots: &[PathBuf]) -> Vec<String> {
    let mut replacements = Vec::new();
    for root in roots {
        if root.is_absolute() {
            replacements.push(root.to_string_lossy().to_string());
        }
        if let Ok(canonical) = std::fs::canonicalize(root) {
            replacements.push(canonical.to_string_lossy().to_string());
        }
    }
    replacements.retain(|root| root.len() > 1);
    replacements.sort_by_key(|root| std::cmp::Reverse(root.len()));
    replacements.dedup();
    replacements
}

fn replace_model_roots(value: &mut String, roots: &[String]) -> bool {
    let original = value.clone();
    for root in roots {
        for candidate in [root.clone(), root.replace('\\', "/")] {
            let slash_prefix = format!("{}/", candidate.trim_end_matches('/'));
            *value = value.replace(&slash_prefix, "instance://");
            *value = value.replace(&candidate, "instance://");
        }
    }
    *value != original
}

fn redact_paths_in_json(value: &mut serde_json::Value, roots: &[String]) -> bool {
    match value {
        serde_json::Value::String(text) => {
            let sanitized = sanitize_private_text(text);
            let mut redacted = sanitized != *text;
            *text = sanitized;
            redacted |= replace_model_roots(text, roots);
            redacted
        }
        serde_json::Value::Array(items) => items.iter_mut().fold(false, |redacted, item| {
            redacted | redact_paths_in_json(item, roots)
        }),
        serde_json::Value::Object(map) => {
            let entries = std::mem::take(map);
            let mut redacted = false;
            for (mut key, mut item) in entries {
                let original_key = key.clone();
                let key_redacted =
                    redact_local_paths(&mut key) | replace_model_roots(&mut key, roots);
                if key_redacted {
                    key.push('#');
                    key.push_str(&stable_hex(&original_key));
                }
                redacted |= key_redacted;
                redacted |= redact_paths_in_json(&mut item, roots);
                map.insert(key, item);
            }
            redacted
        }
        _ => false,
    }
}

fn sanitize_model_location(value: &mut String, roots: &[String]) -> bool {
    let original = value.clone();
    replace_model_roots(value, roots);
    if is_absolute_model_path(value) {
        *value = opaque_path_uri(value);
    }
    *value != original
}

fn is_absolute_model_path(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("file://") || value.starts_with("~/") || value.starts_with("~\\") {
        return true;
    }
    if value.contains("://") {
        return false;
    }
    Path::new(value).is_absolute()
        || value
            .as_bytes()
            .get(1..3)
            .is_some_and(|pair| pair[0] == b':' && matches!(pair[1], b'/' | b'\\'))
        || value.starts_with("\\\\")
}

fn redact_local_paths(value: &mut String) -> bool {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut copied_until = 0usize;
    let mut index = 0usize;
    let mut redacted = false;
    while index < bytes.len() {
        let Some(end) = local_path_end(bytes, index) else {
            index += 1;
            continue;
        };
        output.extend_from_slice(&bytes[copied_until..index]);
        output.extend_from_slice(LOCAL_PATH_REDACTION.as_bytes());
        copied_until = end;
        index = end;
        redacted = true;
    }
    if redacted {
        output.extend_from_slice(&bytes[copied_until..]);
        *value = String::from_utf8(output).expect("path redaction preserves UTF-8");
    }
    redacted
}

fn local_path_end(bytes: &[u8], start: usize) -> Option<usize> {
    if !is_path_boundary(bytes, start) {
        return None;
    }
    let rest = &bytes[start..];
    if starts_ascii_case_insensitive(rest, b"file://") {
        return Some(scan_path_end(bytes, start + 7));
    }
    if rest.starts_with(b"\\\\") {
        return Some(scan_path_end(bytes, start + 2));
    }
    if rest.starts_with(b"~/") || rest.starts_with(b"~\\") {
        return Some(scan_path_end(bytes, start + 2));
    }
    if rest.len() >= 3
        && rest[0].is_ascii_alphabetic()
        && rest[1] == b':'
        && matches!(rest[2], b'/' | b'\\')
    {
        return Some(scan_path_end(bytes, start + 3));
    }
    if rest.first() != Some(&b'/') || rest.get(1) == Some(&b'/') {
        return None;
    }
    let end = scan_path_end(bytes, start + 1);
    bytes[start + 1..end].contains(&b'/').then_some(end)
}

fn is_path_boundary(bytes: &[u8], start: usize) -> bool {
    start == 0
        || !bytes[start - 1].is_ascii()
        || bytes[start - 1].is_ascii_whitespace()
        || matches!(
            bytes[start - 1],
            b'(' | b'[' | b'{' | b'"' | b'\'' | b'`' | b'=' | b':' | b',' | b';'
        )
}

fn scan_path_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() {
        if matches!(
            bytes[end],
            b'"' | b'\''
                | b'`'
                | b'<'
                | b'>'
                | b'('
                | b')'
                | b'['
                | b']'
                | b'{'
                | b'}'
                | b','
                | b';'
        ) {
            break;
        }
        if bytes[end].is_ascii_whitespace() {
            if bytes[end] == b' ' && path_continues_after_space(bytes, end) {
                end += 1;
                continue;
            }
            break;
        }
        end += 1;
    }
    end
}

fn path_continues_after_space(bytes: &[u8], start: usize) -> bool {
    bytes[start + 1..]
        .iter()
        .take_while(|byte| {
            !matches!(
                **byte,
                b'\n'
                    | b'\r'
                    | b'\t'
                    | b'"'
                    | b'\''
                    | b'`'
                    | b'<'
                    | b'>'
                    | b'('
                    | b')'
                    | b'['
                    | b']'
                    | b'{'
                    | b'}'
                    | b','
                    | b';'
            )
        })
        .any(|byte| matches!(*byte, b'/' | b'\\'))
}

fn starts_ascii_case_insensitive(value: &[u8], prefix: &[u8]) -> bool {
    value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn opaque_path_uri(path: &str) -> String {
    format!("opaque://{}", stable_hex(path))
}

#[cfg(test)]
#[path = "privacy_tests.rs"]
mod tests;
