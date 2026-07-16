use super::*;

#[test]
fn line_redaction_handles_nested_and_multiline_secrets_without_hiding_gameplay_tokens() {
    let content = concat!(
        "Quest token: create:crushing_wheel\n",
        "config: api_key=inline-secret\n",
        "const discordToken = `first-secret\n",
        "escaped \\` marker\n",
        "second-secret\n",
        "`\n",
        "password=continued-first\\\n",
        "continued-second\n",
        "credentials:\n",
        "  nested_value: empty-parent-secret\n",
        "  password:\n",
        "  \"same-indent-secret\"\n",
        "  safe_after: visible\n",
        "Header example: Bearer abc.def-credential\n",
        "CLI key: sk-abcdefghijklmnop\n",
        "Stripe key: sk_live_51ExampleLongCredential\n",
        "safe_setting=enabled\n",
    );
    let redacted = sanitize_private_text(content);
    assert!(redacted.contains("Quest token: create:crushing_wheel"));
    assert!(redacted.contains("safe_setting=enabled"));
    for secret in [
        "inline-secret",
        "first-secret",
        "second-secret",
        "continued-first",
        "continued-second",
        "empty-parent-secret",
        "abc.def-credential",
        "sk-abcdefghijklmnop",
        "sk_live_51ExampleLongCredential",
    ] {
        assert!(!redacted.contains(secret), "secret leaked: {secret}");
    }
}

#[test]
fn empty_sensitive_assignment_redacts_a_same_indent_value() {
    let redacted = redact_sensitive_key_lines(concat!(
        "{\n",
        "  password:\n",
        "  \"same-indent-secret: still-secret\"\n",
        "  safe_after: visible\n",
        "}\n",
    ));
    assert!(!redacted.contains("same-indent-secret"));
    assert!(!redacted.contains("still-secret"));
    assert!(redacted.contains("safe_after: visible"));
    assert!(redacted.contains('}'));
}

#[test]
fn local_path_redaction_handles_unicode_boundaries_and_spaces() {
    let mut content = concat!(
        "本地路径：/Users/alice/.ssh/id_rsa, ",
        "C:\\Documents and Settings\\Alice\\secret.txt, ",
        "/Volumes/My Private Pack/config/token.txt; ",
        "/give @p minecraft:diamond",
    )
    .to_string();
    assert!(redact_local_paths(&mut content));
    assert!(!content.contains("alice"));
    assert!(!content.contains("Documents and Settings"));
    assert!(!content.contains("My Private Pack"));
    assert!(content.contains("/give @p minecraft:diamond"));
}

#[test]
fn model_metadata_redacts_credential_bearing_filenames_without_collisions() {
    let mut first = "config/sk-abcdefghijklmnop.txt".to_string();
    let mut second = "config/sk-qrstuvwxyzabcdef.txt".to_string();
    assert!(sanitize_model_identity(&mut first, &[]));
    assert!(sanitize_model_identity(&mut second, &[]));
    assert!(!first.contains("abcdefghijklmnop"));
    assert!(!second.contains("qrstuvwxyzabcdef"));
    assert_ne!(first, second);
}

#[test]
fn private_text_redacts_credentials_after_filename_separators() {
    let redacted = sanitize_private_text(concat!(
        "config/backup-sk-abcdefghijklmnop.txt\n",
        "config/backup_sk_qrstuvwxyzabcdef.txt\n",
    ));
    assert!(!redacted.contains("sk-abcdefghijklmnop"));
    assert!(!redacted.contains("sk_qrstuvwxyzabcdef"));
    assert!(redacted.contains("backup-"));
    assert!(redacted.contains("backup_"));
}

#[test]
fn private_text_redacts_single_and_double_quoted_multiline_values() {
    let redacted = sanitize_private_text(concat!(
        "PRIVATE_KEY=\"-----BEGIN PRIVATE KEY-----\n",
        "MIIE-double-private-material\n",
        "-----END PRIVATE KEY-----\"\n",
        "api_secret='first-single-secret\n",
        "second-single-secret\n",
        "'\n",
        "safe_setting=visible\n",
    ));
    for secret in [
        "MIIE-double-private-material",
        "first-single-secret",
        "second-single-secret",
        "END PRIVATE KEY",
    ] {
        assert!(!redacted.contains(secret), "secret leaked: {secret}");
    }
    assert!(redacted.contains("safe_setting=visible"));
}

#[test]
fn private_text_redacts_unindented_yaml_sensitive_sequences() {
    let redacted = sanitize_private_text(concat!(
        "credentials:\n",
        "- first-secret\n",
        "- second-secret\n",
        "safe: visible\n",
    ));
    assert!(!redacted.contains("first-secret"));
    assert!(!redacted.contains("second-secret"));
    assert!(redacted.contains("safe: visible"));
}

#[test]
fn structured_path_redaction_covers_object_keys() {
    let mut value = serde_json::json!({
        "raw": {
            "/Users/alice/private/config.json": "alice",
            "/Users/bob/private/config.json": "bob"
        }
    });
    assert!(redact_paths_in_json(&mut value, &[]));
    let serialized = serde_json::to_string(&value).unwrap();
    assert!(!serialized.contains("/Users/alice"));
    assert!(!serialized.contains("/Users/bob"));
    assert!(serialized.contains(LOCAL_PATH_REDACTION));
    assert_eq!(value["raw"].as_object().unwrap().len(), 2);
}
