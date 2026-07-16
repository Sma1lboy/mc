use super::{
    agent_history_metadata, agent_history_owned_by, agent_history_visible_to,
    owner_after_session_error, public_share_payload,
};

#[test]
fn public_share_boundary_keeps_only_redacted_display_parts() {
    let raw = serde_json::json!({ "messages": [{ "id": "a", "role": "assistant", "parts": [
        { "type": "reasoning", "text": "private" },
        { "type": "tool-wiki_open", "output": { "content": "secret" } },
        { "type": "text", "text": "visible /Users/alice/private https://example.com token=share-secret" },
        { "type": "tool-ask_user_question", "input": { "question": "Choose", "options": [
            { "label": "A", "description": "safe", "secret": "drop" }
        ] }, "output": { "selected": ["A"], "token": "drop" } }
    ] }] });
    let serialized = serde_json::to_string(&public_share_payload(&raw).unwrap()).unwrap();
    assert!(serialized.contains("visible") && serialized.contains("Choose"));
    assert!(!serialized.contains("private") && !serialized.contains("wiki_open"));
    assert!(!serialized.contains("share-secret") && !serialized.contains("example.com"));
    assert!(!serialized.contains("token") && !serialized.contains("/Users/alice"));
    assert!(!serialized.contains("\"id\""));
}

#[test]
fn local_history_visibility_excludes_other_accounts() {
    let alice = serde_json::json!({ "ownerId": "alice" });
    let anonymous = serde_json::json!({ "ownerId": null });
    assert!(agent_history_owned_by(&alice, "alice"));
    assert!(!agent_history_owned_by(&alice, "bob"));
    assert!(agent_history_visible_to(&alice, Some("alice")));
    assert!(!agent_history_visible_to(&alice, Some("bob")));
    assert!(!agent_history_visible_to(&alice, None));
    assert!(agent_history_visible_to(&anonymous, Some("bob")));
    assert!(agent_history_visible_to(&anonymous, None));
}

#[test]
fn stored_history_metadata_preserves_native_owner() {
    let raw = serde_json::json!({
        "id": "chat-1",
        "updatedAt": 42,
        "ownerId": "alice",
        "messages": [],
    })
    .to_string();

    let (id, updated_at, record) = agent_history_metadata(&raw).unwrap();
    assert_eq!(id, "chat-1");
    assert_eq!(updated_at, 42);
    assert_eq!(
        mc_core::agent::conversation_privacy::conversation_record_owner(&record),
        Some("alice")
    );
}

#[test]
fn session_errors_only_declassify_owner_on_explicit_auth_failure() {
    let unavailable = mc_core::error::CoreError::other("server unavailable");
    assert_eq!(
        owner_after_session_error(&unavailable, Some(Some("alice".into()))).unwrap(),
        Some("alice".into())
    );
    assert!(owner_after_session_error(&unavailable, None).is_err());

    let logged_out = mc_core::error::CoreError::Auth("session expired".into());
    assert_eq!(
        owner_after_session_error(&logged_out, Some(Some("alice".into()))).unwrap(),
        Some("alice".into())
    );
    assert_eq!(owner_after_session_error(&logged_out, None).unwrap(), None);
}
