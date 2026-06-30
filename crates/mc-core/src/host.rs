//! 共享的 host 解析与白名单后缀匹配 —— 下载相关代码的单一所有者。
//!
//! 下载层曾在三处各自手写同一套 `scheme://host` 解析(剥 scheme、丢 `user@` userinfo、
//! 丢 `:port`)与「等于白名单项或为其子域」的反伪造后缀匹配:CurseForge 鉴权判定、
//! mrpack 导入源过滤、mrpack 导出远程引用门。这里收敛成两个纯函数:
//! [`host_of`] 负责解析,[`host_matches_suffix`] 负责后缀匹配。
//!
//! **大小写**:本模块按字节比较,不做归一。各调用点按自身历史语义自行决定是否先
//! `to_ascii_lowercase()`(CurseForge / mrpack 导出归一为小写;mrpack 导入按字节,
//! 与各自重构前行为保持一致)。

/// 从 URL 取出 host:剥离 scheme、丢弃 `user@` userinfo、丢弃 `:port` 与路径 / 查询 / 锚点。
/// 解析不出 host(无 `://`、authority 为空)返回 `None`。**不做大小写归一**。
///
/// 不引入 `url` crate:手解析 `scheme://[userinfo@]host[:port]/...` 的 authority 段。
/// (下载源里不含 IPv6 字面量,无需处理 `[::1]`。)
pub fn host_of(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest)?;
    // authority 到第一个 '/'、'?'、'#' 为止。
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    // 去掉可能的 userinfo(`user:pass@host`,取最后一个 '@' 之后)。
    let host_port = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    // 去掉端口(IPv6 字面量不在我们的下载源里,无需处理 `[::1]`)。
    let host = host_port.split(':').next().unwrap_or(host_port);
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// `host` 是否命中白名单 `allow`:**完全等于**某项,或为其**子域**(以 `.<项>` 结尾)。
///
/// 这条「等于或子域」规则是反 host 后缀伪造的关键:`github.com.evil.com` 不会命中
/// `github.com`(它以 `.evil.com` 结尾,既不等于 `github.com` 也不是其 `.github.com` 子域);
/// 同理 `evilforgecdn.net` 不会命中 `forgecdn.net`(缺少 `.` 分隔)。
/// 按字节比较 —— 调用方负责按自身语义先做大小写归一。
pub fn host_matches_suffix(host: &str, allow: &[&str]) -> bool {
    allow
        .iter()
        .any(|w| host == *w || host.ends_with(&format!(".{w}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_strips_scheme_userinfo_and_port() {
        assert_eq!(
            host_of("https://edge.forgecdn.net/files/1/2/sodium.jar"),
            Some("edge.forgecdn.net")
        );
        assert_eq!(
            host_of("https://api.curseforge.com/v1/mods/files"),
            Some("api.curseforge.com")
        );
        // userinfo + port 都被丢弃。
        assert_eq!(
            host_of("http://user:pass@mediafilez.forgecdn.net:8080/x"),
            Some("mediafilez.forgecdn.net")
        );
        assert_eq!(
            host_of("https://user@cdn.modrinth.com:443/x.jar"),
            Some("cdn.modrinth.com")
        );
        // 查询串不算 host 的一部分。
        assert_eq!(
            host_of("https://cdn.modrinth.com/data/x/y.jar?foo=bar"),
            Some("cdn.modrinth.com")
        );
        // 无 scheme / 无 host → None。
        assert_eq!(host_of("not-a-url"), None);
        assert_eq!(host_of("cdn.modrinth.com/x.jar"), None);
        assert_eq!(host_of("https:///path-only"), None);
    }

    #[test]
    fn suffix_match_allows_exact_and_subdomains() {
        let allow = &["cdn.modrinth.com", "github.com", "forgecdn.net"];
        // 完全等于。
        assert!(host_matches_suffix("cdn.modrinth.com", allow));
        assert!(host_matches_suffix("github.com", allow));
        // 子域。
        assert!(host_matches_suffix("edge.forgecdn.net", allow));
        assert!(host_matches_suffix("media.forge.cdn.modrinth.com", allow));
    }

    #[test]
    fn suffix_match_rejects_spoofed_suffix() {
        let allow = &["github.com", "forgecdn.net", "curseforge.com"];
        // 反伪造:以白名单项「结尾」但不是其子域 → 不命中(无 `.` 分隔)。
        assert!(!host_matches_suffix("evilforgecdn.net", allow));
        assert!(!host_matches_suffix("notgithub.com", allow));
        // 以白名单项「开头」再接恶意域 → 不命中。
        assert!(!host_matches_suffix("github.com.evil.com", allow));
        assert!(!host_matches_suffix("forgecdn.net.evil.com", allow));
        // 完全无关。
        assert!(!host_matches_suffix("example.com", allow));
        assert!(!host_matches_suffix("cdn.modrinth.com", allow));
    }

    #[test]
    fn empty_allowlist_matches_nothing() {
        assert!(!host_matches_suffix("cdn.modrinth.com", &[]));
    }
}
