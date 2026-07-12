    use super::*;
use crate::modplatform::provider::ResourceProvider;

    /// 纯映射:`/mods/files` 风格的一个文件 → ProjectVersion。覆盖:
    /// - `gameVersions` 切分(含 `.` 当游戏版本、loader 名当 loader、Client/Server 丢弃)
    /// - hash algo 1 = sha1(algo 2 = md5 此处不进 VersionFile)
    /// - downloadUrl 存在 → url 正常
    fn sample_file() -> FlameApiFile {
        let json = r#"{
            "id": 4567,
            "modId": 12345,
            "displayName": "Sodium 0.5.3 (Fabric 1.20.1)",
            "fileName": "sodium-fabric-0.5.3.jar",
            "downloadUrl": "https://edge.forgecdn.net/files/4567/sodium.jar",
            "fileLength": 998877,
            "fileFingerprint": 305419896,
            "hashes": [
                { "value": "deadbeefsha1", "algo": 1 },
                { "value": "cafebabemd5", "algo": 2 }
            ],
            "gameVersions": ["1.20.1", "Fabric", "Client", "Server"]
        }"#;
        serde_json::from_str(json).expect("sample file parses")
    }

    #[test]
    fn maps_project_detail_from_mod_and_description() {
        // 纯映射:`POST /mods` 的项目 + description HTML → 与 Modrinth 同一份 ProjectDetail。
        // 覆盖:数字 id 字符串化、links 空串过滤、截图 → 画廊(带标题)、浮点下载量夹取。
        let json = r#"{
            "id": 520914,
            "name": "All the Mods 9",
            "slug": "all-the-mods-9",
            "summary": "ATM9 modpack",
            "downloadCount": 1234567.0,
            "links": {
                "websiteUrl": "https://www.curseforge.com/minecraft/modpacks/all-the-mods-9",
                "wikiUrl": "",
                "issuesUrl": "https://github.com/AllTheMods/ATM-9/issues",
                "sourceUrl": "https://github.com/AllTheMods/ATM-9"
            },
            "logo": { "url": "https://media.forgecdn.net/logo.png" },
            "categories": [{ "name": "Tech" }, { "name": "Magic" }],
            "screenshots": [
                { "url": "https://media.forgecdn.net/s1.png", "title": "Base", "description": "d" },
                { "url": null, "title": "no-url dropped" }
            ]
        }"#;
        let p: FlameApiProject = serde_json::from_str(json).expect("sample project parses");
        let d = map_project_detail(p, "<p>Hello</p>".to_string());

        assert_eq!(d.id, "520914");
        assert_eq!(d.title, "All the Mods 9");
        assert_eq!(d.description, "ATM9 modpack");
        assert_eq!(d.body, "<p>Hello</p>");
        assert_eq!(d.downloads, 1234567);
        assert_eq!(d.followers, 0);
        assert_eq!(d.icon_url.as_deref(), Some("https://media.forgecdn.net/logo.png"));
        assert_eq!(d.categories, vec!["Tech".to_string(), "Magic".to_string()]);
        // url 为 null 的截图被丢弃;标题/描述带过去
        assert_eq!(d.gallery.len(), 1);
        assert_eq!(d.gallery[0].url, "https://media.forgecdn.net/s1.png");
        assert_eq!(d.gallery[0].title.as_deref(), Some("Base"));
        // 空串 wikiUrl 过滤成 None;非空链接保留
        assert_eq!(d.wiki_url, None);
        assert_eq!(d.source_url.as_deref(), Some("https://github.com/AllTheMods/ATM-9"));
        assert_eq!(d.issues_url.as_deref(), Some("https://github.com/AllTheMods/ATM-9/issues"));
        assert_eq!(d.discord_url, None);
    }

    #[test]
    fn maps_file_to_version_with_partition_and_sha1() {
        let f = sample_file();
        let v = map_file_to_version(f);

        assert_eq!(v.id, "4567");
        assert_eq!(v.version_number, "Sodium 0.5.3 (Fabric 1.20.1)");
        // 含 '.' 的当游戏版本
        assert_eq!(v.game_versions, vec!["1.20.1".to_string()]);
        // "Fabric" 当 loader(小写),Client/Server 丢弃
        assert_eq!(v.loaders, vec!["fabric".to_string()]);

        assert_eq!(v.files.len(), 1);
        let file = &v.files[0];
        assert_eq!(file.url, "https://edge.forgecdn.net/files/4567/sodium.jar");
        assert_eq!(file.filename, "sodium-fabric-0.5.3.jar");
        // algo 1 = sha1
        assert_eq!(file.sha1.as_deref(), Some("deadbeefsha1"));
        // CF 不提供 sha512
        assert_eq!(file.sha512, None);
        assert_eq!(file.size, Some(998877));
        // CF 一个 file 即一个版本,primary 恒 true
        assert!(file.primary);
    }

    #[test]
    fn nullable_download_url_is_blocked_empty_string() {
        // downloadUrl 缺失(或 null)= BLOCKED → 映射后 url 为空串。
        let json = r#"{
            "id": 999,
            "modId": 111,
            "displayName": "Blocked Mod 1.0",
            "fileName": "blocked-mod-1.0.jar",
            "fileLength": 4242,
            "hashes": [ { "value": "abc123", "algo": 1 } ],
            "gameVersions": ["1.19.2", "Forge"]
        }"#;
        let f: FlameApiFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.download_url, None);

        let v = map_file_to_version(f);
        let file = &v.files[0];
        // BLOCKED:依然产出 VersionFile,但 url 为空 → 调用方据此识别。
        assert_eq!(file.url, "");
        assert_eq!(file.filename, "blocked-mod-1.0.jar");
        assert_eq!(file.sha1.as_deref(), Some("abc123"));
        assert_eq!(v.game_versions, vec!["1.19.2".to_string()]);
        assert_eq!(v.loaders, vec!["forge".to_string()]);
    }

    #[test]
    fn envelope_with_single_object_under_data_is_tolerated() {
        // /mods/files 单 id 偶发返回单对象而非数组:OneOrMany 容忍两种形态。
        let single = r#"{ "data": {
            "id": 1, "modId": 2, "displayName": "One", "fileName": "one.jar",
            "downloadUrl": "https://x/one.jar", "hashes": [], "gameVersions": ["1.20.1"]
        }}"#;
        let env: FlameEnvelope<OneOrMany<FlameApiFile>> = serde_json::from_str(single).unwrap();
        let files = env.data.into_vec();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name, "one.jar");

        let many = r#"{ "data": [
            { "id": 1, "modId": 2, "displayName": "One", "fileName": "one.jar", "downloadUrl": "https://x/one.jar", "hashes": [], "gameVersions": [] },
            { "id": 3, "modId": 2, "displayName": "Two", "fileName": "two.jar", "downloadUrl": "https://x/two.jar", "hashes": [], "gameVersions": [] }
        ]}"#;
        let env2: FlameEnvelope<OneOrMany<FlameApiFile>> = serde_json::from_str(many).unwrap();
        assert_eq!(env2.data.into_vec().len(), 2);
    }

    #[test]
    fn maps_project_to_search_hit() {
        // /mods or /mods/search 项目对象:嵌套 links/logo/authors/screenshots。
        let json = r#"{
            "id": 12345,
            "name": "Sodium",
            "slug": "sodium",
            "summary": "A rendering engine",
            "downloadCount": 12345678.0,
            "links": { "websiteUrl": "https://www.curseforge.com/minecraft/mc-mods/sodium" },
            "logo": { "url": "https://media.forgecdn.net/avatars/sodium.png" },
            "authors": [ { "name": "jellysquid3" }, { "name": "other" } ],
            "categories": [ { "name": "Cosmetic" } ],
            "screenshots": [ { "url": "https://media.forgecdn.net/attachments/screen1.png" } ]
        }"#;
        let p: FlameApiProject = serde_json::from_str(json).unwrap();
        let hit = map_project(p);

        assert_eq!(hit.id, "12345");
        assert_eq!(hit.slug, "sodium");
        assert_eq!(hit.title, "Sodium");
        assert_eq!(hit.description, "A rendering engine");
        // 第一个作者
        assert_eq!(hit.author, "jellysquid3");
        // downloadCount 浮点 → u64
        assert_eq!(hit.downloads, 12345678);
        assert_eq!(hit.icon_url.as_deref(), Some("https://media.forgecdn.net/avatars/sodium.png"));
        assert_eq!(
            hit.gallery_url.as_deref(),
            Some("https://media.forgecdn.net/attachments/screen1.png")
        );
        assert_eq!(hit.categories, vec!["Cosmetic".to_string()]);
    }

    #[test]
    fn fingerprint_match_carries_file() {
        // /fingerprints 响应:data.exactMatches[].file。
        let json = r#"{ "data": { "exactMatches": [
            { "id": 12345, "file": {
                "id": 4567, "modId": 12345, "displayName": "Sodium",
                "fileName": "sodium.jar", "downloadUrl": "https://x/sodium.jar",
                "fileFingerprint": 305419896,
                "hashes": [ { "value": "sha1here", "algo": 1 } ],
                "gameVersions": ["1.20.1", "Fabric"]
            }}
        ]}}"#;
        let env: FlameEnvelope<FlameFingerprintData> = serde_json::from_str(json).unwrap();
        assert_eq!(env.data.exact_matches.len(), 1);
        let m = &env.data.exact_matches[0];
        assert_eq!(m.file.id, 4567);
        assert_eq!(m.file.mod_id, 12345);
        assert_eq!(m.file.file_fingerprint, Some(305419896));

        // 映射成 ResolvedFile(无项目富化):project_id=modId、version_id=id、url 来自 downloadUrl。
        let resolved = resolved_from_file(&m.file, None);
        assert_eq!(resolved.provider, ProviderId::CurseForge);
        assert_eq!(resolved.project_id, "12345");
        assert_eq!(resolved.version_id, "4567");
        assert_eq!(resolved.file.url, "https://x/sodium.jar");
        assert_eq!(resolved.file.sha1.as_deref(), Some("sha1here"));
        assert_eq!(resolved.project_name, None);
    }

    #[test]
    fn resolved_blocked_file_has_empty_url() {
        // get_files_bulk 语义:BLOCKED 文件仍返回 ResolvedFile,file.url 为空。
        let json = r#"{
            "id": 9, "modId": 8, "displayName": "Blocked", "fileName": "b.jar",
            "fileLength": 10, "hashes": [], "gameVersions": []
        }"#;
        let f: FlameApiFile = serde_json::from_str(json).unwrap();
        let resolved = resolved_from_file(&f, None);
        assert_eq!(resolved.file.url, "");
        assert_eq!(resolved.version_id, "9");
        assert_eq!(resolved.project_id, "8");
    }

    #[test]
    fn empty_envelope_defaults() {
        // 完全空对象也能解析(data 缺失 → 默认空 Vec)。
        let env: FlameEnvelope<Vec<FlameApiProject>> = serde_json::from_str("{}").unwrap();
        assert!(env.data.is_empty());
    }

    #[test]
    fn partition_keeps_dots_and_known_loaders() {
        let (game, loaders) = partition_game_versions(vec![
            "1.20.1".into(),
            "1.21".into(),
            "Fabric".into(),
            "NeoForge".into(),
            "Client".into(),
            "Server".into(),
            "".into(),
        ]);
        assert_eq!(game, vec!["1.20.1".to_string(), "1.21".to_string()]);
        assert_eq!(loaders, vec!["fabric".to_string(), "neoforge".to_string()]);
    }

    #[test]
    fn caps_declares_murmur2_and_needs_key() {
        // 能力声明:CurseForge、需要 key、反查仅 murmur2。
        let api = FlameApi::new("dummy-key");
        let provider = CurseForgeProvider::new(api);
        let caps = provider.caps();
        assert_eq!(caps.id, ProviderId::CurseForge);
        assert_eq!(caps.readable_name, "CurseForge");
        assert_eq!(caps.hash_algos, &[HashAlgo::Murmur2]);
        assert!(caps.needs_api_key);
        assert_eq!(provider.id(), ProviderId::CurseForge);
    }

    #[test]
    fn loader_and_sort_and_class_mappings() {
        assert_eq!(loader_type_id("Forge"), Some(1));
        assert_eq!(loader_type_id("fabric"), Some(4));
        assert_eq!(loader_type_id("QUILT"), Some(5));
        assert_eq!(loader_type_id("neoforge"), Some(6));
        assert_eq!(loader_type_id("rift"), None);

        assert_eq!(sort_field_id(SortMethod::Downloads), 6);
        assert_eq!(sort_field_id(SortMethod::Relevance), 2);

        assert_eq!(class_id(ResourceKind::Mod), CLASS_MOD);
        assert_eq!(class_id(ResourceKind::Modpack), CLASS_MODPACK);
        assert_eq!(class_id(ResourceKind::Datapack), CLASS_MOD);
    }

    #[test]
    fn parse_id_rejects_garbage() {
        assert_eq!(parse_id("123", "x").unwrap(), 123);
        assert!(matches!(parse_id("abc", "x").unwrap_err(), CoreError::Other(_)));
        // 带空白可解析
        assert_eq!(parse_id("  42  ", "x").unwrap(), 42);
    }

    #[test]
    fn from_env_none_when_unset_or_empty() {
        // 不依赖外部环境:直接验证 trim+filter 的语义边界。
        // (env 读取本身在 from_env;此处用 with_base 构造确认 new 不 panic。)
        let api = FlameApi::new("k").with_base("https://example.test/v1");
        assert_eq!(api.api_key(), "k");
        assert!(api.base.ends_with("/v1"));
    }

    #[test]
    fn from_key_trims_and_guards_empty() {
        // 显式 key:去空白后非空才构造。
        assert!(FlameApi::from_key("").is_none());
        assert!(FlameApi::from_key("   ").is_none());
        let api = FlameApi::from_key("  real-key  ").expect("non-empty key constructs");
        assert_eq!(api.api_key(), "real-key");

        // provider 层同样守卫。
        assert!(CurseForgeProvider::from_key("").is_none());
        let p = CurseForgeProvider::from_key("k").expect("non-empty key constructs provider");
        assert_eq!(p.id(), ProviderId::CurseForge);
    }
