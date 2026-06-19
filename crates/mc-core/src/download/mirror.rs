//! 下载镜像改写。中国大陆访问 Mojang 官方 CDN 极慢/不稳,BMCLAPI 提供了
//! 全套官方资源的镜像。本模块做纯字符串前缀替换:命中某个官方前缀就把它换成
//! 镜像上对应的前缀,路径部分原样保留。
//!
//! 设计要点:
//! - 规则是有序的 `(from_prefix, to_prefix)` 列表,匹配第一个命中的前缀即返回。
//!   顺序很重要:更长 / 更具体的前缀必须排在前面,否则会被宽泛前缀抢先匹配。
//! - 改写是纯函数、零网络,可单测。
//! - `none()` 返回空规则集,等价于"直连官方源"。

/// 前缀改写表。每条规则把一个官方 URL 前缀映射到镜像前缀。
#[derive(Debug, Clone, Default)]
pub struct MirrorResolver {
    /// 有序的 (from_prefix, to_prefix)。按顺序匹配,命中即停。
    rules: Vec<(String, String)>,
}

impl MirrorResolver {
    /// 空改写器:所有 URL 原样直连官方源。
    pub fn none() -> Self {
        Self { rules: Vec::new() }
    }

    /// 用任意 (from, to) 前缀对构造改写器(供测试或自定义镜像使用)。
    pub fn from_rules(rules: Vec<(String, String)>) -> Self {
        Self { rules }
    }

    /// 预置 BMCLAPI(bmclapi2.bangbang93.com)改写规则。
    ///
    /// 覆盖 Mojang 的全部资源域:
    /// - piston-meta / launchermeta / launcher.mojang.com:版本清单与元数据
    /// - piston-data:客户端/服务端 jar 与日志配置
    /// - libraries.minecraft.net -> /maven:Maven 库
    /// - resources.download.minecraft.net -> /assets:资源对象(按 hash)
    ///
    /// 注意排序:`libraries.minecraft.net` 与 `resources.download.minecraft.net`
    /// 映射到镜像下不同子路径(/maven、/assets),其余域共享镜像根路径,所以
    /// 必须把这两条特殊规则放在前面优先匹配。
    pub fn bmclapi() -> Self {
        const BASE: &str = "https://bmclapi2.bangbang93.com";
        let rules = vec![
            // —— 特殊子路径映射(必须最先匹配)——
            // 库:libraries.minecraft.net/<group/...> -> BASE/maven/<group/...>
            (
                "https://libraries.minecraft.net".to_string(),
                format!("{BASE}/maven"),
            ),
            // 资源对象:resources.download.minecraft.net/<2hex>/<hash> -> BASE/assets/<2hex>/<hash>
            (
                "https://resources.download.minecraft.net".to_string(),
                format!("{BASE}/assets"),
            ),
            // —— 元数据 / 数据域:整段挂到镜像根 ——
            // piston-meta:version_manifest、各版本 json。
            ("https://piston-meta.mojang.com".to_string(), BASE.to_string()),
            // piston-data:客户端/服务端 jar、log4j 配置。
            ("https://piston-data.mojang.com".to_string(), BASE.to_string()),
            // 旧版 launchermeta:历史版本清单仍走此域。
            ("https://launchermeta.mojang.com".to_string(), BASE.to_string()),
            // 旧版 launcher.mojang.com:旧资源 / jar。
            ("https://launcher.mojang.com".to_string(), BASE.to_string()),
        ];
        Self { rules }
    }

    /// 若 `url` 命中某条规则前缀,返回改写后的 URL;否则原样返回。
    pub fn rewrite(&self, url: &str) -> String {
        for (from, to) in &self.rules {
            if let Some(rest) = url.strip_prefix(from.as_str()) {
                return format!("{to}{rest}");
            }
        }
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_passes_through() {
        let m = MirrorResolver::none();
        let url = "https://piston-data.mojang.com/v1/objects/abc/client.jar";
        assert_eq!(m.rewrite(url), url);
    }

    #[test]
    fn bmclapi_rewrites_libraries_to_maven() {
        let m = MirrorResolver::bmclapi();
        assert_eq!(
            m.rewrite("https://libraries.minecraft.net/com/foo/bar/1.0/bar-1.0.jar"),
            "https://bmclapi2.bangbang93.com/maven/com/foo/bar/1.0/bar-1.0.jar"
        );
    }

    #[test]
    fn bmclapi_rewrites_assets_resources() {
        let m = MirrorResolver::bmclapi();
        assert_eq!(
            m.rewrite("https://resources.download.minecraft.net/ab/abcdef0123"),
            "https://bmclapi2.bangbang93.com/assets/ab/abcdef0123"
        );
    }

    #[test]
    fn bmclapi_rewrites_meta_and_data_to_root() {
        let m = MirrorResolver::bmclapi();
        assert_eq!(
            m.rewrite("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"),
            "https://bmclapi2.bangbang93.com/mc/game/version_manifest_v2.json"
        );
        assert_eq!(
            m.rewrite("https://piston-data.mojang.com/v1/objects/hash/client.jar"),
            "https://bmclapi2.bangbang93.com/v1/objects/hash/client.jar"
        );
        assert_eq!(
            m.rewrite("https://launcher.mojang.com/v1/objects/x/y.jar"),
            "https://bmclapi2.bangbang93.com/v1/objects/x/y.jar"
        );
    }

    #[test]
    fn unknown_host_unchanged() {
        let m = MirrorResolver::bmclapi();
        let url = "https://maven.fabricmc.net/net/fabric/foo.jar";
        assert_eq!(m.rewrite(url), url);
    }
}
