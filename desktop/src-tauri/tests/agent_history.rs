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

fn frontend_canonical_key(record: &str) -> Vec<u16> {
    serde_json::to_string(&serde_json::from_str::<serde_json::Value>(record).unwrap())
        .unwrap()
        .encode_utf16()
        .collect()
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

#[test]
fn upsert_uses_deterministic_whole_record_winner() {
    let forward_path = temp_db_path("deterministic-forward");
    let reverse_path = temp_db_path("deterministic-reverse");
    let loser = r#"{"id":"chat-tie","title":"same time a","createdAt":1,"updatedAt":7,"messages":[{"id":"loser-message","role":"user","parts":[]}],"toolContext":{"mode":"wiki","root":"loser-root"}}"#;
    let winner = r#"{"id":"chat-tie","title":"same time z","createdAt":1,"updatedAt":7,"messages":[{"id":"winner-message","role":"assistant","parts":[]}],"toolContext":{"mode":"instance","root":"winner-root"}}"#;
    let older = r#"{"id":"chat-tie","title":"older","createdAt":1,"updatedAt":6,"messages":[],"toolContext":{"root":"older-root"}}"#;

    // ASCII object keys make serde_json's sorted serialization identical to
    // the frontend: recursively sort keys, stringify, compare UTF-16 units.
    assert!(frontend_canonical_key(winner) > frontend_canonical_key(loser));

    let forward = AgentHistoryStore::open(&forward_path).unwrap();
    forward.upsert("chat-tie", loser).unwrap();
    forward.upsert("chat-tie", winner).unwrap();

    let reverse = AgentHistoryStore::open(&reverse_path).unwrap();
    reverse.upsert("chat-tie", winner).unwrap();
    reverse.upsert("chat-tie", loser).unwrap();

    assert_eq!(forward.load_all().unwrap(), vec![winner.to_string()]);
    assert_eq!(reverse.load_all().unwrap(), vec![winner.to_string()]);

    reverse.upsert("chat-tie", older).unwrap();
    assert_eq!(reverse.load_all().unwrap(), vec![winner.to_string()]);

    reverse.upsert("chat-tie", winner).unwrap();
    assert_eq!(reverse.load_all().unwrap(), vec![winner.to_string()]);

    let _ = std::fs::remove_dir_all(forward_path.parent().unwrap());
    let _ = std::fs::remove_dir_all(reverse_path.parent().unwrap());
}

#[test]
fn upsert_matches_javascript_utf16_string_ordering() {
    let path = temp_db_path("javascript-utf16-order");
    let scalar_order_winner =
        r#"{"id":"chat-unicode","title":"𐀀","createdAt":1,"updatedAt":7,"messages":[]}"#;
    let javascript_winner =
        r#"{"id":"chat-unicode","title":"","createdAt":1,"updatedAt":7,"messages":[]}"#;

    // JavaScript compares UTF-16 code units, so U+E000 sorts after U+10000:
    // 0xE000 > the supplementary character's leading surrogate 0xD800.
    assert!(
        frontend_canonical_key(javascript_winner) > frontend_canonical_key(scalar_order_winner)
    );

    let store = AgentHistoryStore::open(&path).unwrap();
    store.upsert("chat-unicode", javascript_winner).unwrap();
    store.upsert("chat-unicode", scalar_order_winner).unwrap();
    assert_eq!(
        store.load_all().unwrap(),
        vec![javascript_winner.to_string()]
    );

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn opening_legacy_schema_backfills_canonical_record_keys() {
    let path = temp_db_path("canonical-key-migration");
    let winner = r#"{"id":"chat-legacy-schema","title":"z","createdAt":1,"updatedAt":7,"messages":[],"toolContext":{"root":"winner-root"}}"#;
    let loser = r#"{"id":"chat-legacy-schema","title":"a","createdAt":1,"updatedAt":7,"messages":[],"toolContext":{"root":"loser-root"}}"#;
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE agent_conversations (
           id TEXT PRIMARY KEY NOT NULL,
           title TEXT NOT NULL,
           updated_at_ms INTEGER NOT NULL,
           record_json TEXT NOT NULL
         );
         CREATE TABLE agent_history_meta (
           key TEXT PRIMARY KEY NOT NULL,
           value TEXT NOT NULL
         );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO agent_conversations (id, title, updated_at_ms, record_json)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["chat-legacy-schema", "z", 7, winner],
    )
    .unwrap();
    drop(conn);

    let store = AgentHistoryStore::open(&path).unwrap();
    store.upsert("chat-legacy-schema", loser).unwrap();
    assert_eq!(store.load_all().unwrap(), vec![winner.to_string()]);

    let conn = rusqlite::Connection::open(&path).unwrap();
    let key_length: i64 = conn
        .query_row(
            "SELECT length(canonical_record_key) FROM agent_conversations
             WHERE id = 'chat-legacy-schema'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(key_length > 0);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
