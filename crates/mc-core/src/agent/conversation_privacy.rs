use serde_json::{Map, Value};

use crate::error::{CoreError, Result};

use super::tools::sanitize_private_text;

pub const CONVERSATION_PRIVACY_VERSION: u64 = 1;

pub fn sanitize_conversation_text(text: &str) -> String {
    sanitize_persisted_text(text)
}

const REDACTED: &str = "[REDACTED]";
const WIKI_TOOL_TYPES: [&str; 2] = ["tool-wiki_search", "tool-wiki_open"];
const LARGE_PRIVATE_KEYS: [&str; 9] = [
    "raw",
    "body",
    "logtail",
    "manifest",
    "downloadurl",
    "sha1",
    "sha256",
    "sha512",
    "sessionid",
];

pub fn project_conversation_record(record: &Value, owner_id: Option<&str>) -> Result<Value> {
    let object = record
        .as_object()
        .ok_or_else(|| CoreError::other("conversation record must be an object"))?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| CoreError::other("conversation record requires id"))?;
    let messages = object
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::other("conversation record requires messages"))?;

    let mut projected = Map::new();
    projected.insert("id".into(), id.into());
    for key in ["createdAt", "updatedAt"] {
        if let Some(value) = object.get(key) {
            projected.insert(key.into(), value.clone());
        }
    }
    projected.insert(
        "title".into(),
        object
            .get("title")
            .and_then(Value::as_str)
            .map(sanitize_persisted_text)
            .unwrap_or_default()
            .into(),
    );
    projected.insert(
        "messages".into(),
        messages
            .iter()
            .filter_map(project_message)
            .collect::<Vec<_>>()
            .into(),
    );
    projected.insert(
        "toolContext".into(),
        object
            .get("toolContext")
            .and_then(project_tool_context)
            .unwrap_or(Value::Null),
    );
    projected.insert(
        "ownerId".into(),
        owner_id.map(Value::from).unwrap_or(Value::Null),
    );
    projected.insert("privacyVersion".into(), CONVERSATION_PRIVACY_VERSION.into());
    Ok(Value::Object(projected))
}

pub fn project_public_share(payload: &Value) -> Result<Value> {
    let messages = payload
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::other("public share payload requires messages"))?;
    let messages = messages
        .iter()
        .filter_map(|message| {
            let role = message.get("role")?.as_str()?;
            if role != "user" && role != "assistant" {
                return None;
            }
            let parts = message
                .get("parts")?
                .as_array()?
                .iter()
                .filter_map(project_public_part)
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| {
                serde_json::json!({
                    "role": role,
                    "parts": parts,
                })
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "messages": messages }))
}

pub fn conversation_record_owner(record: &Value) -> Option<&str> {
    record.get("ownerId").and_then(Value::as_str)
}

pub fn conversation_record_visible_to(record: &Value, owner_id: Option<&str>) -> bool {
    match (conversation_record_owner(record), owner_id) {
        (None, _) => true,
        (Some(record_owner), Some(owner)) => record_owner == owner,
        (Some(_), None) => false,
    }
}

fn project_message(message: &Value) -> Option<Value> {
    let object = message.as_object()?;
    let mut projected = Map::new();
    for (key, value) in object {
        if key == "parts" || key == "providerMetadata" {
            continue;
        }
        insert_sanitized_entry(
            &mut projected,
            key,
            sanitize_value(value, key, 0, false),
            false,
        );
    }
    let parts = object
        .get("parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(project_part)
        .collect::<Vec<_>>();
    projected.insert("parts".into(), parts.into());
    Some(Value::Object(projected))
}

fn project_part(part: &Value) -> Option<Value> {
    let object = part.as_object()?;
    let part_type = object.get("type").and_then(Value::as_str);
    if part_type == Some("reasoning") {
        return None;
    }
    let mut projected = sanitize_value(part, "part", 0, false);
    if part_type.is_some_and(|kind| WIKI_TOOL_TYPES.contains(&kind)) {
        if let Some(object) = projected.as_object_mut() {
            if object.contains_key("output") {
                object.insert(
                    "output".into(),
                    serde_json::json!({
                        "privacyRedacted": true,
                        "reason": "instance_content_not_persisted",
                    }),
                );
            }
        }
    }
    Some(projected)
}

fn project_tool_context(context: &Value) -> Option<Value> {
    let object = context.as_object()?;
    let mut projected = Map::new();
    copy_string(object, &mut projected, "mode");
    if let Some(instance) = object.get("instance").and_then(Value::as_object) {
        projected.insert(
            "instance".into(),
            select_strings(
                instance,
                &["modpackId", "instanceId", "mcVersion", "loader"],
            ),
        );
    } else if let Some(wiki) = object.get("wiki").and_then(Value::as_object) {
        projected.insert(
            "wiki".into(),
            select_strings(wiki, &["modpackId", "instanceId"]),
        );
    }
    Some(Value::Object(projected))
}

fn select_strings(object: &Map<String, Value>, keys: &[&str]) -> Value {
    let mut selected = Map::new();
    for key in keys {
        copy_string(object, &mut selected, key);
    }
    Value::Object(selected)
}

fn copy_string(source: &Map<String, Value>, target: &mut Map<String, Value>, key: &str) {
    if let Some(value) = source.get(key).and_then(Value::as_str) {
        target.insert(key.into(), sanitize_persisted_text(value).into());
    }
}

fn project_public_part(part: &Value) -> Option<Value> {
    let object = part.as_object()?;
    match object.get("type")?.as_str()? {
        "text" => Some(serde_json::json!({
            "type": "text",
            "text": sanitize_public_text(object.get("text")?.as_str()?),
        })),
        "tool-ask_user_question" => project_public_question(object),
        _ => None,
    }
}

fn project_public_question(part: &Map<String, Value>) -> Option<Value> {
    let input = part.get("input")?.as_object()?;
    let question = sanitize_public_text(input.get("question")?.as_str()?);
    let options = input
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| {
            let option = option.as_object()?;
            let mut projected = Map::new();
            projected.insert(
                "label".into(),
                sanitize_public_text(option.get("label")?.as_str()?).into(),
            );
            if let Some(description) = option.get("description").and_then(Value::as_str) {
                projected.insert(
                    "description".into(),
                    sanitize_public_text(description).into(),
                );
            }
            Some(Value::Object(projected))
        })
        .collect::<Vec<_>>();
    let selected = part
        .get("output")
        .and_then(|output| output.get("selected"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(sanitize_public_text)
        .collect::<Vec<_>>();
    Some(serde_json::json!({
        "type": "tool-ask_user_question",
        "input": { "question": question, "options": options },
        "output": { "selected": selected },
    }))
}

fn sanitize_value(value: &Value, key: &str, depth: usize, public: bool) -> Value {
    if depth > 24 || is_sensitive_key(key) || is_large_private_key(key) {
        return REDACTED.into();
    }
    match value {
        Value::String(text) => {
            if public {
                sanitize_public_text(text).into()
            } else {
                sanitize_persisted_text(text).into()
            }
        }
        Value::Array(items) => items
            .iter()
            .map(|item| sanitize_value(item, key, depth + 1, public))
            .collect::<Vec<_>>()
            .into(),
        Value::Object(object) => {
            let mut sanitized = Map::new();
            for (entry_key, entry_value) in object {
                if entry_key == "providerMetadata" {
                    continue;
                }
                insert_sanitized_entry(
                    &mut sanitized,
                    entry_key,
                    sanitize_value(entry_value, entry_key, depth + 1, public),
                    public,
                );
            }
            Value::Object(sanitized)
        }
        _ => value.clone(),
    }
}

fn insert_sanitized_entry(target: &mut Map<String, Value>, key: &str, value: Value, public: bool) {
    let base = if public {
        sanitize_public_text(key)
    } else {
        sanitize_persisted_text(key)
    };
    let mut candidate = base.clone();
    let mut suffix = 2;
    while target.contains_key(&candidate) {
        candidate = format!("{base}#{suffix}");
        suffix += 1;
    }
    target.insert(candidate, value);
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = normalized_key(key);
    [
        "password",
        "passwd",
        "passphrase",
        "secret",
        "token",
        "accesstoken",
        "refreshtoken",
        "apikey",
        "authorization",
        "auth",
        "credential",
        "privatekey",
        "webhook",
        "cookie",
        "databaseurl",
        "connectionstring",
    ]
    .iter()
    .any(|needle| normalized == *needle || normalized.ends_with(needle))
}

fn is_large_private_key(key: &str) -> bool {
    let normalized = normalized_key(key);
    LARGE_PRIVATE_KEYS.contains(&normalized.as_str()) || normalized == "outputpath"
}

fn normalized_key(key: &str) -> String {
    key.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn sanitize_persisted_text(text: &str) -> String {
    redact_hashes(&redact_urls(&sanitize_private_text(text)))
}

fn sanitize_public_text(text: &str) -> String {
    sanitize_persisted_text(text)
}

fn redact_urls(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut copied_until = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        let Some(mut end) = url_prefix_end(bytes, index) else {
            index += 1;
            continue;
        };
        let mut parenthesis_depth = 0usize;
        while end < bytes.len() {
            let byte = bytes[end];
            if byte.is_ascii_whitespace() || matches!(byte, b'"' | b'\'' | b'`' | b'<' | b'>') {
                break;
            }
            match byte {
                b'(' => parenthesis_depth += 1,
                b')' if parenthesis_depth == 0 => break,
                b')' => parenthesis_depth -= 1,
                _ => {}
            }
            end += 1;
        }
        output.extend_from_slice(&bytes[copied_until..index]);
        output.extend_from_slice(REDACTED.as_bytes());
        copied_until = end;
        index = end;
    }
    if copied_until == 0 {
        return text.to_string();
    }
    output.extend_from_slice(&bytes[copied_until..]);
    String::from_utf8(output).expect("URL redaction preserves UTF-8")
}

fn url_prefix_end(bytes: &[u8], start: usize) -> Option<usize> {
    if !bytes.get(start).is_some_and(u8::is_ascii_alphabetic)
        || start > 0
            && matches!(
                bytes[start - 1],
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'+' | b'-' | b'.'
            )
    {
        return None;
    }
    let mut end = start + 1;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    {
        end += 1;
    }
    bytes
        .get(end..end + 3)
        .is_some_and(|suffix| suffix == b"://")
        .then_some(end + 3)
}

fn redact_hashes(text: &str) -> String {
    let text = redact_labelled_digests(text);
    let text = text.as_str();
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut copied_until = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        if !bytes[index].is_ascii_hexdigit() || index > 0 && bytes[index - 1].is_ascii_hexdigit() {
            index += 1;
            continue;
        }
        let mut end = index;
        while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
            end += 1;
        }
        if end - index >= 40 {
            output.extend_from_slice(&bytes[copied_until..index]);
            output.extend_from_slice(REDACTED.as_bytes());
            copied_until = end;
        }
        index = end;
    }
    if copied_until == 0 {
        return text.to_string();
    }
    output.extend_from_slice(&bytes[copied_until..]);
    String::from_utf8(output).expect("hash redaction preserves UTF-8")
}

fn redact_labelled_digests(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut copied_until = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        if index > 0 && bytes[index - 1].is_ascii_alphanumeric() {
            index += 1;
            continue;
        }
        let Some(label_len) = [b"sha512".as_slice(), b"sha256", b"sha1"]
            .into_iter()
            .find(|label| starts_ascii_case_insensitive(&bytes[index..], label))
            .map(|label| label.len())
        else {
            index += 1;
            continue;
        };
        let separator = index + label_len;
        if !bytes
            .get(separator)
            .is_some_and(|byte| matches!(byte, b':' | b'-'))
        {
            index += label_len;
            continue;
        }
        let token_start = separator + 1;
        let mut end = token_start;
        while bytes.get(end).is_some_and(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'/' | b'=' | b':' | b'-')
        }) {
            end += 1;
        }
        if end - token_start < 20 {
            index = end.max(token_start);
            continue;
        }
        output.extend_from_slice(&bytes[copied_until..index]);
        output.extend_from_slice(REDACTED.as_bytes());
        copied_until = end;
        index = end;
    }
    if copied_until == 0 {
        return text.to_string();
    }
    output.extend_from_slice(&bytes[copied_until..]);
    String::from_utf8(output).expect("labelled digest redaction preserves UTF-8")
}

fn starts_ascii_case_insensitive(value: &[u8], prefix: &[u8]) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_projection_rebuilds_a_safe_continuation_record() {
        let record = serde_json::json!({
            "id": "chat-1",
            "createdAt": 1,
            "updatedAt": 2,
            "title": "guide /Users/alice/private",
            "ownerId": "mallory",
            "privacyVersion": 999,
            "toolContext": {
                "mode": "instance",
                "root": "/Users/alice/Games",
                "instance": {
                    "root": "/Users/alice/Games",
                    "sourcePaths": ["/Users/alice/Games/pack"],
                    "modpackId": "pack",
                    "instanceId": "instance",
                    "mcVersion": "1.20.1",
                    "loader": "fabric"
                }
            },
            "messages": [{
                "id": "message-1",
                "role": "assistant",
                "parts": [
                    { "type": "reasoning", "text": "private chain" },
                    { "type": "text", "text": "https://storage.example/signed?token=secret 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" },
                    { "type": "tool-wiki_open", "toolCallId": "call-1", "output": {
                        "password": "secret", "content": "/Users/alice/private"
                    } }
                ]
            }]
        });
        let projected = project_conversation_record(&record, Some("alice")).unwrap();
        let serialized = serde_json::to_string(&projected).unwrap();
        assert!(!serialized.contains("private chain"));
        assert!(!serialized.contains("storage.example"));
        assert!(!serialized.contains("0123456789abcdef0123456789abcdef"));
        assert!(!serialized.contains("/Users/alice"));
        assert!(!serialized.contains("\"password\":\"secret\""));
        assert!(serialized.contains("toolCallId"));
        assert!(serialized.contains("instance_content_not_persisted"));
        assert_eq!(projected["ownerId"], "alice");
        assert_eq!(projected["privacyVersion"], 1);
        assert!(projected["toolContext"]["instance"].get("root").is_none());
    }

    #[test]
    fn public_projection_is_display_only_and_redacts_strings() {
        let raw = serde_json::json!({ "messages": [{
            "id": "message-1",
            "role": "assistant",
            "parts": [
                { "type": "reasoning", "text": "private" },
                { "type": "tool-wiki_open", "output": { "content": "secret" } },
                { "type": "text", "text": "See /Users/alice/private at https://example.com/a token=share-secret Bearer abc.def sk-abcdefghijklmnop sk_live_51ExampleLongCredential" },
                { "type": "tool-ask_user_question", "input": {
                    "question": "Choose /Users/alice/private",
                    "options": [{ "label": "A", "description": "https://secret.example" }]
                }, "output": { "selected": ["A"], "token": "drop" } }
            ]
        }] });
        let projected = project_public_share(&raw).unwrap();
        let serialized = serde_json::to_string(&projected).unwrap();
        assert!(serialized.contains("Choose"));
        assert!(!serialized.contains("message-1"));
        assert!(!serialized.contains("wiki_open"));
        assert!(!serialized.contains("/Users/alice"));
        assert!(!serialized.contains("example.com"));
        assert!(!serialized.contains("share-secret"));
        assert!(!serialized.contains("abc.def"));
        assert!(!serialized.contains("sk-abcdefghijklmnop"));
        assert!(!serialized.contains("sk_live_51ExampleLongCredential"));
        assert!(!serialized.contains("\"token\""));
    }

    #[test]
    fn persisted_text_redacts_generic_urls_and_arbitrarily_long_hashes() {
        let text = format!("s3://private-bucket/object {}", "a".repeat(160));
        let sanitized = sanitize_persisted_text(&text);
        assert!(!sanitized.contains("private-bucket"));
        assert!(!sanitized.contains(&"a".repeat(160)));
    }

    #[test]
    fn conversation_text_redacts_parenthesized_urls_and_labelled_digests() {
        let text = concat!(
            "See https://internal.example/report(user)?sig=private-value next; ",
            "sha256:QWxhZGRpbjpvcGVuIHNlc2FtZV9wcml2YXRlLXNpZw== ",
            "sha512-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-private ",
            "gameplay=create:crushing_wheel ",
            "unlabelled=QWxhZGRpbjpvcGVuIHNlc2FtZQ==",
        );

        let sanitized = sanitize_conversation_text(text);
        assert!(!sanitized.contains("internal.example"));
        assert!(!sanitized.contains("user)?sig=private-value"));
        assert!(!sanitized.contains("QWxhZGRpbjpvcGVuIHNlc2FtZV9wcml2YXRlLXNpZw"));
        assert!(!sanitized.contains("AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-private"));
        assert!(sanitized.contains("next"));
        assert!(sanitized.contains("create:crushing_wheel"));
        assert!(sanitized.contains("QWxhZGRpbjpvcGVuIHNlc2FtZQ=="));
    }

    #[test]
    fn persistence_projection_sanitizes_object_keys_without_dropping_collisions() {
        let mut metadata = Map::new();
        metadata.insert("https://one.example/private".into(), "first".into());
        metadata.insert("https://two.example/private".into(), "second".into());
        metadata.insert("a".repeat(40), "third".into());
        metadata.insert("b".repeat(40), "fourth".into());
        let record = serde_json::json!({
            "id": "chat-keys",
            "messages": [{
                "role": "assistant",
                "parts": [{ "type": "text", "text": "safe" }],
                "metadata": metadata,
            }],
        });

        let projected = project_conversation_record(&record, None).unwrap();
        let metadata = projected["messages"][0]["metadata"].as_object().unwrap();
        let serialized = serde_json::to_string(metadata).unwrap();
        assert_eq!(metadata.len(), 4);
        for key in [REDACTED, "[REDACTED]#2", "[REDACTED]#3", "[REDACTED]#4"] {
            assert!(metadata.contains_key(key));
        }
        let mut values = metadata
            .values()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        values.sort_unstable();
        assert_eq!(values, ["first", "fourth", "second", "third"]);
        assert!(!serialized.contains("one.example"));
        assert!(!serialized.contains("two.example"));
        assert!(!serialized.contains(&"a".repeat(40)));
        assert!(!serialized.contains(&"b".repeat(40)));
    }

    #[test]
    fn visibility_never_exposes_another_accounts_records() {
        let anonymous = serde_json::json!({ "ownerId": null });
        let alice = serde_json::json!({ "ownerId": "alice" });
        assert!(conversation_record_visible_to(&anonymous, Some("bob")));
        assert!(!conversation_record_visible_to(&alice, Some("bob")));
        assert!(conversation_record_visible_to(&alice, Some("alice")));
        assert!(!conversation_record_visible_to(&alice, None));
    }

    #[test]
    fn sensitive_key_matching_does_not_hide_author_metadata() {
        let record = serde_json::json!({
            "id": "chat-author",
            "messages": [{
                "role": "assistant",
                "author": "Guide Writer",
                "apiToken": "private-token",
                "parts": [{ "type": "text", "text": "safe" }],
            }],
        });

        let projected = project_conversation_record(&record, None).unwrap();
        assert_eq!(projected["messages"][0]["author"], "Guide Writer");
        assert_eq!(projected["messages"][0]["apiToken"], REDACTED);
    }
}
