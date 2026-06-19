//! 离线账号会话生成。
//!
//! 离线账号没有任何网络验证:用户名直接决定一个稳定的 UUID。我们沿用
//! Java 版客户端/服务端的离线命名惯例(`OfflinePlayer:<name>` 的 MD5),
//! 这样同一个用户名在任意启动器、任意服务器上都得到相同的 UUID,玩家进入
//! 离线服务器时身份是一致的。

use mc_types::AuthSession;

use md5::{Digest, Md5};

/// 根据用户名生成一个离线 [`AuthSession`]。
///
/// UUID 算法与 Java 的 `UUID.nameUUIDFromBytes` 一致:
/// 1. 计算 `md5("OfflinePlayer:" + username)`,得到 16 字节。
/// 2. 按 RFC 4122 设置 version (3, name-based MD5) 与 variant 位。
/// 3. 格式化为带连字符的 8-4-4-4-12 小写十六进制字符串。
///
/// `access_token` 给占位 `"0"`(离线无真实 token),`user_type` 为 `legacy`,
/// `xuid` 为空(仅 Microsoft 账号有 xuid)。
pub fn offline_session(username: &str) -> AuthSession {
    let uuid = offline_uuid(username);
    AuthSession {
        username: username.to_string(),
        uuid,
        access_token: "0".to_string(),
        user_type: "legacy".to_string(),
        xuid: String::new(),
    }
}

/// 计算离线 UUID(带连字符,小写)。等价于 Java
/// `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name).getBytes(UTF_8))`。
pub fn offline_uuid(username: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(b"OfflinePlayer:");
    hasher.update(username.as_bytes());
    let mut bytes: [u8; 16] = hasher.finalize().into();

    // 设置 version=3(name-based, MD5):清高 4 位再置 0b0011。
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    // 设置 variant=RFC4122:高两位置为 0b10。
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format_uuid(&bytes)
}

/// 将 16 字节格式化为标准带连字符的 UUID 字符串。
fn format_uuid(bytes: &[u8; 16]) -> String {
    let hex = hex::encode(bytes);
    // 8-4-4-4-12 分组。
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_uuid_for_fixed_username() {
        // 固定用户名必须产出稳定 UUID(回归保护)。
        // 该值由 Java UUID.nameUUIDFromBytes("OfflinePlayer:Notch") 验证。
        let s = offline_session("Notch");
        assert_eq!(s.uuid, "b50ad385-829d-3141-a216-7e7d7539ba7f");
        assert_eq!(s.username, "Notch");
        assert_eq!(s.access_token, "0");
        assert_eq!(s.user_type, "legacy");
        assert!(s.xuid.is_empty());
    }

    #[test]
    fn uuid_format_is_canonical() {
        let u = offline_uuid("Steve");
        // 形如 8-4-4-4-12,共 36 字符,4 个连字符。
        assert_eq!(u.len(), 36);
        assert_eq!(u.matches('-').count(), 4);
        let groups: Vec<&str> = u.split('-').collect();
        assert_eq!(
            groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        // version nibble 必须为 '3'。
        assert_eq!(&u[14..15], "3");
        // variant nibble 必须在 8..=b。
        let variant = u8::from_str_radix(&u[19..20], 16).unwrap();
        assert!((0x8..=0xb).contains(&variant));
    }

    #[test]
    fn different_usernames_differ() {
        assert_ne!(offline_uuid("alice"), offline_uuid("bob"));
    }

    #[test]
    fn same_username_is_deterministic() {
        assert_eq!(offline_uuid("Player"), offline_uuid("Player"));
    }
}
