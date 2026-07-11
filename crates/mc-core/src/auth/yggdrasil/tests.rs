    use super::*;
    use std::path::PathBuf;

    #[test]
    fn javaagent_arg_concatenates_path_and_base() {
        let path = PathBuf::from("/opt/mc/authlib-injector.jar");
        let arg = javaagent_arg(&path, "https://littleskin.cn/api/yggdrasil");
        assert_eq!(
            arg,
            "-javaagent:/opt/mc/authlib-injector.jar=https://littleskin.cn/api/yggdrasil"
        );
    }

    #[test]
    fn javaagent_arg_trims_trailing_slash_on_base() {
        let path = PathBuf::from("/a/b.jar");
        let arg = javaagent_arg(&path, "https://example.com/api/yggdrasil///");
        assert_eq!(arg, "-javaagent:/a/b.jar=https://example.com/api/yggdrasil");
    }

    #[test]
    fn client_javaagent_arg_uses_normalized_base() {
        let client = YggdrasilClient::new("https://littleskin.cn/api/yggdrasil/");
        let arg = client.javaagent_arg(Path::new("/x/injector.jar"));
        assert_eq!(
            arg,
            "-javaagent:/x/injector.jar=https://littleskin.cn/api/yggdrasil"
        );
    }

    #[test]
    fn new_normalizes_base() {
        let c = YggdrasilClient::new("https://host/api/yggdrasil/");
        assert_eq!(c.base(), "https://host/api/yggdrasil");
        let c2 = YggdrasilClient::new("https://host/api/yggdrasil");
        assert_eq!(c2.base(), "https://host/api/yggdrasil");
    }

    #[test]
    fn endpoint_joins_without_double_slash() {
        let c = YggdrasilClient::new("https://host/api/yggdrasil/");
        assert_eq!(
            c.endpoint("authserver/authenticate"),
            "https://host/api/yggdrasil/authserver/authenticate"
        );
        // 即使路径带前导斜杠也不会出现双斜杠。
        assert_eq!(
            c.endpoint("/authserver/validate"),
            "https://host/api/yggdrasil/authserver/validate"
        );
    }

    #[test]
    fn stable_client_token_is_deterministic_and_hex() {
        let a = stable_client_token("alice");
        let b = stable_client_token("alice");
        assert_eq!(a, b, "同一用户名必须得到相同 clientToken");
        assert_eq!(a.len(), 32, "应为 32 位 hex");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        // 不同用户名应不同。
        assert_ne!(stable_client_token("alice"), stable_client_token("bob"));
    }

    #[test]
    fn parse_session_reads_selected_profile() {
        // 模拟 authenticate 成功响应。
        let v = json!({
            "accessToken": "AT",
            "clientToken": "CT",
            "selectedProfile": {
                "id": "4566e69fc90748ee8a1015c8b41d1c00",
                "name": "Steve"
            },
            "availableProfiles": [
                { "id": "4566e69fc90748ee8a1015c8b41d1c00", "name": "Steve" }
            ]
        });
        let s = parse_session(&v, "ignored", "REQ_CT").unwrap();
        assert_eq!(s.access_token, "AT");
        // 响应回带 clientToken 时优先用它。
        assert_eq!(s.client_token, "CT");
        assert_eq!(s.username, "Steve");
        // id 被转成带连字符形式。
        assert_eq!(s.uuid, "4566e69f-c907-48ee-8a10-15c8b41d1c00");
    }

    #[test]
    fn parse_session_falls_back_to_available_profiles() {
        // 没有 selectedProfile 时取 availableProfiles 第一个。
        let v = json!({
            "accessToken": "AT",
            "availableProfiles": [
                { "id": "00000000000000000000000000000001", "name": "First" },
                { "id": "00000000000000000000000000000002", "name": "Second" }
            ]
        });
        let s = parse_session(&v, "fallback", "REQ_CT").unwrap();
        assert_eq!(s.username, "First");
        // 响应没回带 clientToken 时用请求值兜底。
        assert_eq!(s.client_token, "REQ_CT");
    }

    #[test]
    fn parse_session_uses_request_client_token_when_absent() {
        let v = json!({
            "accessToken": "AT",
            "selectedProfile": { "id": "ab", "name": "X" }
        });
        let s = parse_session(&v, "", "REQ_CT").unwrap();
        assert_eq!(s.client_token, "REQ_CT");
        // 非 32 位 id 原样保留。
        assert_eq!(s.uuid, "ab");
    }

    #[test]
    fn parse_session_errors_on_missing_access_token() {
        let v = json!({ "selectedProfile": { "id": "ab", "name": "X" } });
        assert!(parse_session(&v, "", "CT").is_err());
    }

    #[test]
    fn parse_session_errors_when_no_profile() {
        let v = json!({ "accessToken": "AT", "clientToken": "CT" });
        let err = parse_session(&v, "", "CT").unwrap_err();
        match err {
            CoreError::Auth(m) => assert!(m.contains("角色"), "应提示无可用角色: {m}"),
            other => panic!("期望 Auth 错误,得到 {other:?}"),
        }
    }

    #[test]
    fn parse_session_uses_fallback_username_when_profile_name_empty() {
        let v = json!({
            "accessToken": "AT",
            "clientToken": "CT",
            "selectedProfile": { "id": "ab", "name": "" }
        });
        let s = parse_session(&v, "FallbackName", "CT").unwrap();
        assert_eq!(s.username, "FallbackName");
    }

    #[test]
    fn yggdrasil_error_parses_error_message() {
        // 典型的凭据错误响应。
        let v = json!({
            "error": "ForbiddenOperationException",
            "errorMessage": "Invalid credentials. Invalid username or password."
        });
        let err = yggdrasil_error(403, &v, "外置登录");
        match err {
            CoreError::Auth(m) => {
                assert!(m.contains("外置登录失败"), "前缀缺失: {m}");
                assert!(m.contains("用户名或密码错误"), "应有中文提示: {m}");
                assert!(m.contains("Invalid credentials"), "应透传原始消息: {m}");
            }
            other => panic!("期望 Auth 错误,得到 {other:?}"),
        }
    }

    #[test]
    fn yggdrasil_error_passes_through_unknown_message() {
        let v = json!({
            "error": "SomeOtherException",
            "errorMessage": "皮肤站维护中"
        });
        let err = yggdrasil_error(500, &v, "校验登录态");
        match err {
            CoreError::Auth(m) => {
                assert!(m.contains("校验登录态失败"));
                assert!(m.contains("皮肤站维护中"));
            }
            other => panic!("期望 Auth 错误,得到 {other:?}"),
        }
    }

    #[test]
    fn yggdrasil_error_falls_back_to_status_when_empty() {
        // 完全没有 JSON 错误体(代理回了空)。
        let err = yggdrasil_error(502, &Value::Null, "外置登录");
        match err {
            CoreError::Auth(m) => {
                assert!(m.contains("502"), "应带状态码: {m}");
                assert!(m.contains("外置登录失败"));
            }
            other => panic!("期望 Auth 错误,得到 {other:?}"),
        }
    }

    #[test]
    fn to_auth_session_maps_fields() {
        let sess = YggdrasilSession {
            access_token: "AT".into(),
            client_token: "CT".into(),
            uuid: "4566e69f-c907-48ee-8a10-15c8b41d1c00".into(),
            username: "Steve".into(),
        };
        let a = sess.to_auth_session();
        assert_eq!(a.username, "Steve");
        assert_eq!(a.uuid, "4566e69f-c907-48ee-8a10-15c8b41d1c00");
        assert_eq!(a.access_token, "AT");
        // 外置登录归一为 msa,xuid 空。
        assert_eq!(a.user_type, "msa");
        assert!(a.xuid.is_empty());
    }

    #[test]
    fn dashify_roundtrip() {
        assert_eq!(
            dashify_uuid("4566e69fc90748ee8a1015c8b41d1c00"),
            "4566e69f-c907-48ee-8a10-15c8b41d1c00"
        );
        // 已带连字符:原样。
        let dashed = "4566e69f-c907-48ee-8a10-15c8b41d1c00";
        assert_eq!(dashify_uuid(dashed), dashed);
        // 长度异常:原样。
        assert_eq!(dashify_uuid("abc"), "abc");
        // 非 hex 的 32 字符串:原样(不强行分组)。
        assert_eq!(dashify_uuid(&"z".repeat(32)), "z".repeat(32));
    }

    #[test]
    fn dashify_non_ascii_32_bytes_does_not_panic() {
        // 异常/恶意皮肤站可能返回一个**恰好 32 字节**但含多字节字符的角色 id;
        // 旧实现只检查 `len()==32` 后按字节切片,会在非字符边界 panic。
        // 构造一个 32 字节、含跨切片边界(byte 8)多字节字符的串,断言原样透传不 panic。
        let raw = format!("{}{}{}", "a".repeat(7), "é", "b".repeat(23));
        assert_eq!(raw.len(), 32);
        assert_eq!(dashify_uuid(&raw), raw);
    }
