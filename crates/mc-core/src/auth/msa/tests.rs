    use super::*;

    #[test]
    fn dashify_inserts_hyphens() {
        assert_eq!(
            dashify_uuid("4566e69fc90748ee8a1015c8b41d1c00"),
            "4566e69f-c907-48ee-8a10-15c8b41d1c00"
        );
    }

    #[test]
    fn dashify_passthrough_when_already_dashed() {
        let dashed = "4566e69f-c907-48ee-8a10-15c8b41d1c00";
        assert_eq!(dashify_uuid(dashed), dashed);
    }

    #[test]
    fn dashify_non_ascii_32_bytes_does_not_panic() {
        // 异常/恶意服务端可能返回一个**恰好 32 字节**但含多字节字符的 id;
        // 旧实现只检查 `len()==32` 后按字节切片,会在非字符边界 panic。
        // 这里构造一个 32 字节、含跨切片边界(byte 8)的多字节字符的串,断言原样透传不 panic。
        let raw = format!("{}{}{}", "a".repeat(7), "é", "b".repeat(23));
        assert_eq!(raw.len(), 32);
        assert_eq!(dashify_uuid(&raw), raw);
    }

    #[test]
    fn extract_uhs_reads_nested_claim() {
        let v = json!({"DisplayClaims": {"xui": [{"uhs": "abc123"}]}});
        assert_eq!(extract_uhs(&v).as_deref(), Some("abc123"));
    }

    #[test]
    fn extract_uhs_missing_returns_none() {
        assert!(extract_uhs(&json!({"DisplayClaims": {"xui": []}})).is_none());
        assert!(extract_uhs(&json!({})).is_none());
    }

    #[test]
    fn xsts_hint_translates_known_codes() {
        assert!(xsts_hint(2148916233).contains("Xbox"));
        assert!(xsts_hint(2148916235).contains("地区"));
        assert!(xsts_hint(2148916238).contains("未成年"));
        assert!(xsts_hint(999).contains("999"));
    }

    #[test]
    fn parse_token_extracts_fields() {
        let v = json!({
            "access_token": "AT",
            "refresh_token": "RT",
            "expires_in": 3600
        });
        let t = parse_token(&v).unwrap();
        assert_eq!(t.access_token, "AT");
        assert_eq!(t.refresh_token, "RT");
        assert_eq!(t.expires_in, 3600);
    }

    #[test]
    fn parse_token_rejects_missing_access_token() {
        assert!(parse_token(&json!({"refresh_token": "RT"})).is_err());
    }
