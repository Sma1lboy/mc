    use super::*;
    use sqlx::PgPool;

    async fn mk_user(pool: &PgPool, id: &str, email: &str) {
        sqlx::query("INSERT INTO users (id, email) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING")
            .bind(id)
            .bind(email)
            .execute(pool)
            .await
            .unwrap();
    }

    fn req(name: &str) -> CreateRealmReq {
        CreateRealmReq {
            name: name.into(),
            expires_in_secs: None,
            manifest: RealmManifest { mc_version: Some("1.20.1".into()), loader: Some("fabric".into()), ..Default::default() },
        }
    }

    #[tokio::test]
    async fn realm_lifecycle_and_permissions() {
        let Some(pool) = crate::db::test_pool().await else { return };
        // Clean slate for this test's fixed users (cascades realms/members).
        sqlx::query("DELETE FROM users WHERE id IN ('t-realm-owner', 't-realm-friend')")
            .execute(&pool)
            .await
            .unwrap();
        mk_user(&pool, "t-realm-owner", "owner@test.local").await;
        mk_user(&pool, "t-realm-friend", "friend@test.local").await;

        let store = RealmStore::new(pool.clone());

        // create → owner enrolled, manifest at v1
        let realm = store.create("t-realm-owner", &req("Survival")).await.unwrap();
        assert_eq!(realm.role, "owner");
        assert_eq!(realm.manifest_version, 1);
        assert_eq!(realm.mc_version.as_deref(), Some("1.20.1"));
        assert_eq!(realm.code.len(), 6);

        // join by code → friend is a member
        let joined = store.join("t-realm-friend", &realm.code).await.unwrap().unwrap();
        assert_eq!(joined.id, realm.id);
        assert_eq!(joined.role, "member");
        // unknown code → None
        assert!(store.join("t-realm-friend", "ZZZZZZ").await.unwrap().is_none());

        // member CANNOT push the manifest
        let m = RealmManifest {
            mc_version: Some("1.20.1".into()),
            loader: Some("fabric".into()),
            loader_version: None,
            files: vec![RealmFile {
                path: "mods/sodium.jar".into(),
                sha1: Some("abc".into()),
                sha512: None,
                size: Some(10),
                url: Some("https://cdn/sodium.jar".into()),
                source: Some("modrinth".into()),
            }],
            overrides: None,
            source: None,
            version: 0,
        };
        assert!(store.push_manifest(&realm.id, "t-realm-friend", &m).await.unwrap().is_none());

        // owner CAN push → version bumps to 2
        assert_eq!(store.push_manifest(&realm.id, "t-realm-owner", &m).await.unwrap(), Some(2));

        // member reads the manifest (files + server version)
        let got = store.manifest(&realm.id, "t-realm-friend").await.unwrap().unwrap();
        assert_eq!(got.version, 2);
        assert_eq!(got.files.len(), 1);
        assert_eq!(got.files[0].path, "mods/sodium.jar");

        // promote friend → admin; now they can push
        assert!(store.set_role(&realm.id, "t-realm-owner", "t-realm-friend", "admin").await.unwrap());
        assert_eq!(store.push_manifest(&realm.id, "t-realm-friend", &m).await.unwrap(), Some(3));

        // members list shows both; mark-synced records progress
        store.mark_synced(&realm.id, "t-realm-friend", 3).await.unwrap();
        let members = store.members(&realm.id, "t-realm-owner").await.unwrap().unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.iter().any(|m| m.user_id == "t-realm-friend" && m.synced_version == 3));

        // host publish (P3): nobody hosting yet → (None, None)
        assert_eq!(store.get_host(&realm.id).await.unwrap(), (None, None));
        // owner publishes a host address → fresh read returns it
        store.set_host(&realm.id, "t-realm-owner", "10.144.0.1:52137").await.unwrap();
        let (addr, _name) = store.get_host(&realm.id).await.unwrap();
        assert_eq!(addr.as_deref(), Some("10.144.0.1:52137"));
        // a non-host can't clear it; the host can, and then it reads empty again
        assert!(!store.clear_host(&realm.id, "t-realm-friend").await.unwrap());
        assert!(store.clear_host(&realm.id, "t-realm-owner").await.unwrap());
        assert_eq!(store.get_host(&realm.id).await.unwrap(), (None, None));

        // friend leaves → 1 member; owner can't be removed
        assert!(store.leave_or_remove(&realm.id, "t-realm-friend", "t-realm-friend").await.unwrap());
        assert!(!store.leave_or_remove(&realm.id, "t-realm-owner", "t-realm-owner").await.unwrap());
        assert_eq!(store.list_mine("t-realm-friend").await.unwrap().len(), 0);

        // non-owner can't disband; owner can
        assert!(!store.delete(&realm.id, "t-realm-friend").await.unwrap());
        assert!(store.delete(&realm.id, "t-realm-owner").await.unwrap());
        assert!(store.summary_for(&realm.id, "t-realm-owner").await.unwrap().is_none());

        // cleanup
        sqlx::query("DELETE FROM users WHERE id IN ('t-realm-owner', 't-realm-friend')")
            .execute(&pool)
            .await
            .unwrap();
    }
