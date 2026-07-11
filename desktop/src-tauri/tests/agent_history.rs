use mc_launcher_desktop_lib::agent_history::AgentHistoryStore;

fn temp_db_path(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "mc-agent-history-test-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir_all(&root).unwrap();
    root.join("history.sqlite3")
}

#[test]
fn persists_and_reloads_conversation_records() {
    let path = temp_db_path("roundtrip");
    let record =
        r#"{"id":"chat-1","title":"first question","createdAt":1,"updatedAt":2,"messages":[]}"#;

    AgentHistoryStore::open(&path)
        .unwrap()
        .upsert("chat-1", record)
        .unwrap();

    let records = AgentHistoryStore::open(&path).unwrap().load_all().unwrap();
    assert_eq!(records, vec![record.to_string()]);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn retains_only_the_newest_fifty_conversations() {
    let path = temp_db_path("limit");
    let store = AgentHistoryStore::open(&path).unwrap();
    for i in 0..51 {
        store
            .upsert(
                &format!("chat-{i}"),
                &format!(
                    r#"{{"id":"chat-{i}","title":"chat {i}","createdAt":{i},"updatedAt":{i},"messages":[]}}"#
                ),
            )
            .unwrap();
    }

    let records = store.load_all().unwrap();
    assert_eq!(records.len(), 50);
    assert!(!records
        .iter()
        .any(|record| record.contains("\"id\":\"chat-0\"")));

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn imports_legacy_webkit_localstorage_without_modifying_the_source() {
    let path = temp_db_path("webkit-import");
    let legacy = path.with_file_name("localstorage.sqlite3");
    let source =
        r#"[{"id":"chat-legacy","title":"old chat","createdAt":1,"updatedAt":2,"messages":[]}]"#;
    let utf16 = source
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let conn = rusqlite::Connection::open(&legacy).unwrap();
    conn.execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB NOT NULL)")
        .unwrap();
    conn.execute(
        "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
        rusqlite::params!["mc-launcher.agentConversations", utf16],
    )
    .unwrap();
    drop(conn);

    let store = AgentHistoryStore::open(&path).unwrap();
    assert_eq!(store.import_webkit_database(&legacy).unwrap(), 1);
    let records = store.load_all().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&records[0]).unwrap(),
        serde_json::json!({
            "id": "chat-legacy",
            "title": "old chat",
            "createdAt": 1,
            "updatedAt": 2,
            "messages": [],
        }),
    );

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn persists_records_larger_than_the_cloud_payload_limit() {
    let path = temp_db_path("large-record");
    let record = format!(
        r#"{{"id":"chat-large","title":"diagnostic","createdAt":1,"updatedAt":2,"messages":[{{"role":"assistant","parts":[{{"type":"text","text":"{}"}}]}}]}}"#,
        "x".repeat(1_048_576),
    );
    let store = AgentHistoryStore::open(&path).unwrap();

    store.upsert("chat-large", &record).unwrap();
    assert_eq!(store.load_all().unwrap(), vec![record]);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
