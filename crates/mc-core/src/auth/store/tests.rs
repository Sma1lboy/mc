    use super::*;

    /// `cfg(test)` 下 [`secret`] 后端自动换成进程内内存 store(见该模块),测试绝不触碰真实
    /// 钥匙串。此处保留一个显式的 no-op,标注「这个测试会走 keyring 路径」。
    fn init_mock_keyring() {}

    #[test]
    fn is_expired_respects_margin_and_unknown() {
        let mut a = offline("x", "u");
        a.expires_at = Some(1_000);
        // 1000 到期、margin 60:940 起即视为(接近)过期。
        assert!(!a.is_expired(939, 60));
        assert!(a.is_expired(940, 60));
        assert!(a.is_expired(1_000, 60));
        // expires_at 未知 → 一律视为过期(促使续期)。
        a.expires_at = None;
        assert!(a.is_expired(0, 0));
    }

    fn ms_session() -> AuthSession {
        AuthSession {
            username: "msuser".to_string(),
            uuid: "uuid-ms".to_string(),
            access_token: "acc".to_string(),
            user_type: "msa".to_string(),
            xuid: "x1".to_string(),
        }
    }

    /// 初次登录与续期两条路径用同样的输入(owns_game 对齐)应得到同样的字段布局 + 同样的 TTL。
    /// 这是去重的核心保证:`from_microsoft` 只是 `from_microsoft_refreshed(.., true)` 的薄包装,
    /// 字段布局 / TTL 只此一份。
    #[test]
    fn microsoft_initial_and_refresh_paths_match() {
        let session = ms_session();
        let before = now_unix();
        let initial = StoredAccount::from_microsoft(&session, "refresh".to_string());
        let refreshed =
            StoredAccount::from_microsoft_refreshed(&session, "refresh".to_string(), true);
        let after = now_unix();

        // TTL:两条路径都把 expires_at 设为各自构造时刻 + MC_TOKEN_TTL_SECS。
        for acc in [&initial, &refreshed] {
            let exp = acc.expires_at.expect("微软账号应有 expires_at");
            assert!(exp >= before + MC_TOKEN_TTL_SECS && exp <= after + MC_TOKEN_TTL_SECS);
        }
        // 除 expires_at(各自取构造时刻)外其余字段逐一相同 —— 字段布局只此一份。
        let norm = |mut a: StoredAccount| {
            a.expires_at = None;
            a
        };
        assert_eq!(norm(initial.clone()), norm(refreshed.clone()));

        // 字段布局 spot-check:确实是拥有正版、带 refresh_token、msa 的微软账号,且无外置字段。
        assert_eq!(initial.kind, AccountKind::Microsoft);
        assert!(initial.owns_game);
        assert_eq!(initial.refresh_token.as_deref(), Some("refresh"));
        assert_eq!(initial.user_type, "msa");
        assert_eq!(initial.xuid, "x1");
        assert!(initial.client_token.is_none());
        assert!(initial.yggdrasil_base.is_none());
    }

    /// 续期路径相对初次登录的真实差异:`owns_game` 沿用旧账号(可能为 false),而非强制 `true`。
    #[test]
    fn microsoft_refreshed_carries_owns_game() {
        let acc = StoredAccount::from_microsoft_refreshed(&ms_session(), "r".to_string(), false);
        assert!(!acc.owns_game);
    }

    fn offline(name: &str, uuid: &str) -> StoredAccount {
        StoredAccount {
            kind: AccountKind::Offline,
            username: name.to_string(),
            uuid: uuid.to_string(),
            access_token: "0".to_string(),
            refresh_token: None,
            xuid: String::new(),
            user_type: "legacy".to_string(),
            owns_game: false,
            expires_at: None,
            client_token: None,
            yggdrasil_base: None,
        }
    }

    fn empty_store() -> AccountStore {
        AccountStore {
            path: PathBuf::from("/tmp/does-not-matter.json"),
            accounts: Vec::new(),
            selected: None,
        }
    }

    #[test]
    fn first_added_becomes_selected() {
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        assert_eq!(s.selected.as_deref(), Some("uuid-a"));
        let sess = s.selected_session().unwrap();
        assert_eq!(sess.username, "alice");
        assert_eq!(sess.user_type, "legacy");
    }

    #[test]
    fn add_existing_uuid_replaces_in_place() {
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        s.add(offline("bob", "uuid-b"));
        // 用同一 uuid 再 add,应替换而非新增。
        let mut updated = offline("alice2", "uuid-a");
        updated.access_token = "newtok".to_string();
        s.add(updated);
        assert_eq!(s.accounts.len(), 2);
        let a = s.accounts.iter().find(|a| a.uuid == "uuid-a").unwrap();
        assert_eq!(a.username, "alice2");
        assert_eq!(a.access_token, "newtok");
    }

    #[test]
    fn select_unknown_uuid_errors() {
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        assert!(s.select("nope").is_err());
        assert!(s.select("uuid-a").is_ok());
    }

    #[test]
    fn add_and_select_switches_selection_to_each_new_account() {
        // 落盘到唯一临时路径,避免与并发测试争用固定文件。
        let path = std::env::temp_dir()
            .join(format!("mc-store-addsel-{}.json", std::process::id()));
        let mut s = AccountStore { path, accounts: Vec::new(), selected: None };
        s.add_and_select(offline("alice", "uuid-a")).unwrap();
        s.add_and_select(offline("bob", "uuid-b")).unwrap();
        // `add` 单独只在列表原本为空时自动选中第一个;add_and_select 必须把选中切到
        // **每个**新加入的账号 —— 这正是所有登录调用方依赖、却各自手写易漏的那一步。
        assert_eq!(s.selected.as_deref(), Some("uuid-b"));
    }

    #[test]
    fn remove_selected_falls_back_to_first() {
        init_mock_keyring();
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        s.add(offline("bob", "uuid-b"));
        s.select("uuid-b").unwrap();
        assert!(s.remove("uuid-b"));
        // 选中项回退到剩余列表的第一个。
        assert_eq!(s.selected.as_deref(), Some("uuid-a"));
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        init_mock_keyring();
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        assert!(!s.remove("ghost"));
    }

    #[test]
    fn list_marks_selected() {
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        s.add(offline("bob", "uuid-b"));
        s.select("uuid-b").unwrap();
        let list = s.list();
        let a = list.iter().find(|x| x.uuid == "uuid-a").unwrap();
        let b = list.iter().find(|x| x.uuid == "uuid-b").unwrap();
        assert!(!a.selected);
        assert!(b.selected);
    }

    #[test]
    fn load_missing_file_is_empty() {
        let p = std::env::temp_dir().join("mc-core-auth-store-missing-xyz.json");
        let _ = std::fs::remove_file(&p);
        let s = AccountStore::load(&p).unwrap();
        assert!(s.accounts().is_empty());
        assert!(s.selected_session().is_none());
    }

    #[test]
    fn save_then_load_roundtrips() {
        init_mock_keyring();
        let p = std::env::temp_dir().join(format!(
            "mc-core-auth-store-roundtrip-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);

        let mut s = AccountStore::load(&p).unwrap();
        s.add(offline("alice", "uuid-a"));
        s.add(StoredAccount {
            kind: AccountKind::Microsoft,
            username: "msuser".to_string(),
            uuid: "uuid-ms".to_string(),
            access_token: "mctoken".to_string(),
            refresh_token: Some("refresh".to_string()),
            xuid: "xuid123".to_string(),
            user_type: "msa".to_string(),
            owns_game: true,
            expires_at: None,
            client_token: None,
            yggdrasil_base: None,
        });
        s.select("uuid-ms").unwrap();
        s.save().unwrap();

        // 开启 keyring(mock)时:磁盘上不应再出现明文 token,而是带 keyring 标记。
        #[cfg(feature = "keyring")]
        {
            let on_disk = std::fs::read_to_string(&p).unwrap();
            assert!(
                !on_disk.contains("mctoken") && !on_disk.contains("\"refresh\""),
                "敏感 token 不应明文落盘:{on_disk}"
            );
            assert!(on_disk.contains("secrets_in_keyring"));
        }

        let loaded = AccountStore::load(&p).unwrap();
        assert_eq!(loaded.accounts().len(), 2);
        assert_eq!(loaded.selected.as_deref(), Some("uuid-ms"));
        let sess = loaded.selected_session().unwrap();
        assert_eq!(sess.username, "msuser");
        assert_eq!(sess.user_type, "msa");
        assert_eq!(sess.xuid, "xuid123");
        let ms = loaded
            .accounts()
            .iter()
            .find(|a| a.uuid == "uuid-ms")
            .unwrap();
        assert_eq!(ms.refresh_token.as_deref(), Some("refresh"));
        assert!(ms.owns_game);

        // 含明文 token 的账号库应被收紧为仅属主可读写(0600)。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "accounts.json 应为 0600,实际 {:o}", mode & 0o777);
        }

        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn load_drops_dangling_selected() {
        // 选中项指向不存在的 uuid 时,加载后应回退到第一个账号。
        let p = std::env::temp_dir().join(format!(
            "mc-core-auth-store-dangling-{}.json",
            std::process::id()
        ));
        let json = r#"{
            "accounts": [
                {"kind":"offline","username":"alice","uuid":"uuid-a",
                 "access_token":"0","user_type":"legacy"}
            ],
            "selected": "ghost"
        }"#;
        std::fs::write(&p, json).unwrap();
        let s = AccountStore::load(&p).unwrap();
        assert_eq!(s.selected.as_deref(), Some("uuid-a"));
        let _ = std::fs::remove_file(&p);
    }

    /// 离线账号的占位 token("0")不算秘密:始终留在明文文件里,不进 keyring,
    /// 加载/保存照常往返。feature 开关都成立。
    #[test]
    fn offline_token_stays_plaintext() {
        init_mock_keyring();
        let p = std::env::temp_dir().join(format!(
            "mc-core-auth-store-offline-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);

        let mut s = AccountStore::load(&p).unwrap();
        s.add(offline("alice", "uuid-off"));
        s.save().unwrap();

        let on_disk = std::fs::read_to_string(&p).unwrap();
        assert!(on_disk.contains("\"access_token\": \"0\""));
        assert!(!on_disk.contains("secrets_in_keyring"));

        let loaded = AccountStore::load(&p).unwrap();
        assert_eq!(loaded.selected_session().unwrap().access_token, "0");
        let _ = std::fs::remove_file(&p);
    }

    /// 旧版明文文件(token 在 JSON 里、无 keyring 标记)在加载时应自动迁移:token 进 keyring,
    /// 文件被改写清除明文;之后重新加载仍能从 keyring 取回 token。仅在 keyring feature 开启时成立。
    #[cfg(feature = "keyring")]
    #[test]
    fn legacy_plaintext_migrates_into_keyring() {
        init_mock_keyring();
        let p = std::env::temp_dir().join(format!(
            "mc-core-auth-store-migrate-{}.json",
            std::process::id()
        ));
        let json = r#"{
            "accounts": [
                {"kind":"microsoft","username":"legacy","uuid":"uuid-legacy",
                 "access_token":"plain-access","refresh_token":"plain-refresh",
                 "xuid":"x1","user_type":"msa","owns_game":true}
            ],
            "selected": "uuid-legacy"
        }"#;
        std::fs::write(&p, json).unwrap();

        // 加载触发迁移:内存里 token 仍正确。
        let loaded = AccountStore::load(&p).unwrap();
        let acc = loaded.selected_account().unwrap();
        assert_eq!(acc.access_token, "plain-access");
        assert_eq!(acc.refresh_token.as_deref(), Some("plain-refresh"));

        // 文件已被改写:不再含明文 token,改为 keyring 标记。
        let on_disk = std::fs::read_to_string(&p).unwrap();
        assert!(
            !on_disk.contains("plain-access") && !on_disk.contains("plain-refresh"),
            "迁移后明文 token 不应残留:{on_disk}"
        );
        assert!(on_disk.contains("secrets_in_keyring"));

        // 全新加载:token 从 keyring 取回。
        let reloaded = AccountStore::load(&p).unwrap();
        let acc = reloaded.selected_account().unwrap();
        assert_eq!(acc.access_token, "plain-access");
        assert_eq!(acc.refresh_token.as_deref(), Some("plain-refresh"));

        let _ = std::fs::remove_file(&p);
    }
