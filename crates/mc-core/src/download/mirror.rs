//! 下载镜像改写。中国大陆访问 Mojang / Modrinth / CurseForge 官方 CDN 极慢/不稳:
//! BMCLAPI 镜像全套**游戏**资源,McIM(mod.mcimirror.top)镜像**社区 mod**资源。
//!
//! 设计要点:
//! - 一条规则把一个官方 URL 前缀映射到**一个或多个**镜像前缀(如库在 BMCLAPI 上同时
//!   存在 `/maven` 与 `/libraries` 两个路径,都给出作互为候选)。
//! - [`MirrorResolver::candidates`] 返回**有序候选 URL 列表**(镜像变体 + 官方回退),
//!   下载器据此逐个失败转移;[`MirrorResolver::rewrite`] 返回首选候选(兼容旧接口)。
//! - [`DownloadSource`] 控制官方 / 镜像的先后:国内默认镜像优先 + 官方兜底。
//! - 改写是纯函数、零网络,可单测。`none()` 等价于"直连官方源"。

/// 下载源偏好:决定候选列表里官方与镜像的先后。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DownloadSource {
    /// 自适应(当前等同镜像优先,留作未来按测速动态切换)。
    #[default]
    Auto,
    /// 镜像优先,官方兜底(国内推荐)。
    MirrorFirst,
    /// 官方优先,镜像兜底(网络通畅时更"正统")。
    OfficialFirst,
}

/// 一条镜像规则:`from` 前缀命中后,映射到 `to` 里的每一个镜像前缀(路径原样保留)。
#[derive(Debug, Clone)]
struct MirrorRule {
    from: String,
    to: Vec<String>,
}

/// 镜像改写器。持有有序规则表与源偏好。
#[derive(Debug, Clone, Default)]
pub struct MirrorResolver {
    /// 有序规则,按顺序匹配,命中即停。更具体的前缀须排在更宽泛前缀之前。
    rules: Vec<MirrorRule>,
    source: DownloadSource,
}

const BMCLAPI: &str = "https://bmclapi2.bangbang93.com";
const MCIM: &str = "https://mod.mcimirror.top";

impl MirrorResolver {
    /// 空改写器:所有 URL 原样直连官方源。
    pub fn none() -> Self {
        Self::default()
    }

    /// 用任意 `(from, to)` 前缀对构造(单镜像)。供测试 / 自定义镜像使用。
    pub fn from_rules(rules: Vec<(String, String)>) -> Self {
        Self {
            rules: rules.into_iter().map(|(from, to)| MirrorRule { from, to: vec![to] }).collect(),
            source: DownloadSource::default(),
        }
    }

    /// BMCLAPI(游戏资源):version 清单 / 版本 json / client jar / libraries / assets /
    /// Forge·Fabric·OptiFine 安装器 / Java。
    ///
    /// 库映射到 `/maven` **与** `/libraries` 两个变体(某些工件只在其一),都作候选;
    /// `resources.download` → `/assets`;其余元数据 / 数据域整段挂到镜像根。排序:特殊子
    /// 路径(`/maven`、`/assets`)必须排在共享镜像根的规则之前。
    pub fn bmclapi() -> Self {
        Self {
            source: DownloadSource::default(),
            rules: vec![
                MirrorRule {
                    from: "https://libraries.minecraft.net".into(),
                    to: vec![format!("{BMCLAPI}/maven"), format!("{BMCLAPI}/libraries")],
                },
                MirrorRule {
                    from: "https://resources.download.minecraft.net".into(),
                    to: vec![format!("{BMCLAPI}/assets")],
                },
                MirrorRule { from: "https://piston-meta.mojang.com".into(), to: vec![BMCLAPI.into()] },
                MirrorRule { from: "https://piston-data.mojang.com".into(), to: vec![BMCLAPI.into()] },
                MirrorRule { from: "https://launchermeta.mojang.com".into(), to: vec![BMCLAPI.into()] },
                MirrorRule { from: "https://launcher.mojang.com".into(), to: vec![BMCLAPI.into()] },
            ],
        }
    }

    /// McIM(社区 mod 资源):Modrinth API / CDN、CurseForge API、Forge CDN。
    /// 整合包导入 / mod 浏览下载经它获得国内镜像覆盖。
    pub fn mcim() -> Self {
        Self {
            source: DownloadSource::default(),
            rules: vec![
                MirrorRule { from: "https://api.modrinth.com".into(), to: vec![format!("{MCIM}/modrinth")] },
                MirrorRule { from: "https://staging-api.modrinth.com".into(), to: vec![format!("{MCIM}/modrinth")] },
                MirrorRule { from: "https://cdn.modrinth.com".into(), to: vec![MCIM.into()] },
                MirrorRule { from: "https://api.curseforge.com".into(), to: vec![format!("{MCIM}/curseforge")] },
                MirrorRule { from: "https://edge.forgecdn.net".into(), to: vec![MCIM.into()] },
                MirrorRule { from: "https://mediafilez.forgecdn.net".into(), to: vec![MCIM.into()] },
                MirrorRule { from: "https://media.forgecdn.net".into(), to: vec![MCIM.into()] },
            ],
        }
    }

    /// 国内全套:BMCLAPI(游戏)+ McIM(mod)。整合包场景推荐。
    pub fn china() -> Self {
        let mut base = Self::bmclapi();
        base.rules.extend(Self::mcim().rules);
        base
    }

    /// 设置源偏好(链式)。
    pub fn with_source(mut self, source: DownloadSource) -> Self {
        self.source = source;
        self
    }

    /// 返回 `url` 命中规则后的全部镜像变体(不含官方原址);未命中则空。
    fn mirror_variants(&self, url: &str) -> Vec<String> {
        for rule in &self.rules {
            if let Some(rest) = url.strip_prefix(rule.from.as_str()) {
                return rule.to.iter().map(|to| format!("{to}{rest}")).collect();
            }
        }
        Vec::new()
    }

    /// 返回 `url` 的**有序候选列表**:按源偏好排布镜像变体与官方原址,去重保序。
    /// 未命中任何镜像规则时即 `[url]`。下载器对列表逐个失败转移。
    pub fn candidates(&self, url: &str) -> Vec<String> {
        let mirrored = self.mirror_variants(url);
        let official = url.to_string();
        let mut out: Vec<String> = match self.source {
            DownloadSource::OfficialFirst => {
                let mut v = vec![official];
                v.extend(mirrored);
                v
            }
            // Auto / MirrorFirst:镜像在前,官方兜底。
            _ => {
                let mut v = mirrored;
                v.push(official);
                v
            }
        };
        // 去重保序。
        let mut seen = std::collections::HashSet::new();
        out.retain(|u| seen.insert(u.clone()));
        out
    }

    /// 首选候选 URL(兼容旧接口)。镜像优先时即镜像改写结果,无镜像时即原址。
    pub fn rewrite(&self, url: &str) -> String {
        self.candidates(url).into_iter().next().unwrap_or_else(|| url.to_string())
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
        assert_eq!(m.candidates(url), vec![url.to_string()]);
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
    fn library_has_maven_and_libraries_variants_plus_official_fallback() {
        let m = MirrorResolver::bmclapi();
        let c = m.candidates("https://libraries.minecraft.net/a/b.jar");
        assert_eq!(
            c,
            vec![
                "https://bmclapi2.bangbang93.com/maven/a/b.jar".to_string(),
                "https://bmclapi2.bangbang93.com/libraries/a/b.jar".to_string(),
                "https://libraries.minecraft.net/a/b.jar".to_string(),
            ]
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
            m.rewrite("https://launcher.mojang.com/v1/objects/x/y.jar"),
            "https://bmclapi2.bangbang93.com/v1/objects/x/y.jar"
        );
    }

    #[test]
    fn mcim_rewrites_mod_hosts() {
        let m = MirrorResolver::mcim();
        assert_eq!(
            m.rewrite("https://cdn.modrinth.com/data/AABB/versions/x/sodium.jar"),
            "https://mod.mcimirror.top/data/AABB/versions/x/sodium.jar"
        );
        assert_eq!(
            m.rewrite("https://api.modrinth.com/v2/search?q=sodium"),
            "https://mod.mcimirror.top/modrinth/v2/search?q=sodium"
        );
        assert_eq!(
            m.rewrite("https://edge.forgecdn.net/files/1/2/mod.jar"),
            "https://mod.mcimirror.top/files/1/2/mod.jar"
        );
    }

    #[test]
    fn china_covers_both_game_and_mod() {
        let m = MirrorResolver::china();
        // 游戏(BMCLAPI)
        assert_eq!(
            m.rewrite("https://piston-data.mojang.com/v1/objects/h/client.jar"),
            "https://bmclapi2.bangbang93.com/v1/objects/h/client.jar"
        );
        // mod(McIM)
        assert_eq!(
            m.rewrite("https://cdn.modrinth.com/data/x/y.jar"),
            "https://mod.mcimirror.top/data/x/y.jar"
        );
    }

    #[test]
    fn official_first_puts_official_before_mirror() {
        let m = MirrorResolver::bmclapi().with_source(DownloadSource::OfficialFirst);
        let c = m.candidates("https://piston-data.mojang.com/v1/objects/h/client.jar");
        assert_eq!(c[0], "https://piston-data.mojang.com/v1/objects/h/client.jar");
        assert_eq!(c[1], "https://bmclapi2.bangbang93.com/v1/objects/h/client.jar");
    }

    #[test]
    fn unknown_host_unchanged() {
        let m = MirrorResolver::china();
        let url = "https://maven.fabricmc.net/net/fabric/foo.jar";
        assert_eq!(m.rewrite(url), url);
        assert_eq!(m.candidates(url), vec![url.to_string()]);
    }

    #[test]
    fn from_rules_still_works() {
        let m = MirrorResolver::from_rules(vec![(
            "https://example.com".into(),
            "https://mirror.example.cn".into(),
        )]);
        assert_eq!(m.rewrite("https://example.com/a/b"), "https://mirror.example.cn/a/b");
    }
}
