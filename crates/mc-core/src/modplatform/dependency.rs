//! Provider-agnostic 依赖解析器(移植 PCL-CE `ModDependencyResolver` / Prism 的依赖游走)。
//!
//! 见 `docs/modules/content-providers.md` §3。给定一组「根」引用(`(provider, project_id)`),
//! 在统一的 [`ProviderRegistry`](super::provider::ProviderRegistry) 上做有界 BFS:
//!
//! - 对每个引用,选 provider → `list_versions(project_id, Some(mc), Some(loader))`,
//!   按「精确游戏版本匹配 > loader 匹配 > 第一个兼容版本」挑出最佳版本及其主文件,
//!   产出一个 [`ResolvedFile`](super::ResolvedFile)。
//! - 该版本的 `dependencies`:`required` 入队继续游走,`incompatible` 记入冲突集,
//!   其余(`optional`/`embedded` 等)为劝告性,跳过。
//! - 已装(`already_installed`)的项目直接判为已满足,不再递归。
//! - `visited`(键 = `(provider, project_id)`)去重,使一个 mod 在多条路径上只解析一次;
//!   `MAX_DEPTH = 32` 作为环/失控的硬护栏。
//!
//! **provider 无关 + 纯算法**:网络只藏在 trait 后面,本模块只负责编排与去重,因此可用
//! 一个内存版 `FakeProvider` 完整单测,无需联网。这与 [`crate::modplatform`] 「映射/算法
//! 可单测、IO 在边界」的约定一致。
//!
//! 设计取舍:[`ProjectVersion`](super::ProjectVersion) 不带发布日期,故「最新」无法直接比较;
//! 这里以「第一个 `game_versions` 含 `mc` **且** `loaders` 含 `loader` 的版本」作为合理代理
//! (Modrinth/CurseForge 的版本列表本就按时间倒序返回,第一个兼容项即最新兼容版)。

use std::collections::{HashSet, VecDeque};

use crate::error::Result;

use super::provider::ProviderRegistry;
use super::{ProjectVersion, ProviderId, ResolvedFile};

/// 环 / 失控护栏:依赖游走的最大深度(对齐 PCL-CE `MaxDepth = 32`)。
pub const MAX_DEPTH: usize = 32;

/// 一个跨平台的项目引用 = `(provider, project_id)`。这是依赖图里的去重键
/// (PCL-CE 的 `(Source, ProjectId)`)。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModRef {
    pub provider: ProviderId,
    pub project_id: String,
}

impl ModRef {
    /// 便捷构造。
    pub fn new(provider: ProviderId, project_id: impl Into<String>) -> Self {
        Self { provider, project_id: project_id.into() }
    }

    /// `visited` / `already_installed` 用的稳定字符串键。
    ///
    /// 用 `<provider>:<project_id>` 形式,既能区分不同平台的同名 id,又便于上层把
    /// 「已装清单」用同样的键塞进 `already_installed`(见 [`resolve_dependencies`])。
    pub fn key(&self) -> String {
        format!("{}:{}", provider_tag(self.provider), self.project_id)
    }
}

/// provider 的短标签,用于拼 [`ModRef::key`](ModRef::key)。
fn provider_tag(p: ProviderId) -> &'static str {
    match p {
        ProviderId::Modrinth => "modrinth",
        ProviderId::CurseForge => "curseforge",
    }
}

/// 依赖解析的结果:一次性给「将装什么 / 已满足 / 没找到 / 冲突」四张清单,供 UI 预览,
/// **不静默安装**(对齐设计文档 §3)。
#[derive(Debug, Clone, Default)]
pub struct DepResolution {
    /// 需要下载安装的文件(根 + 递归到的必需依赖),已按 `(provider, project_id)` 去重。
    pub to_install: Vec<ResolvedFile>,
    /// 已被 `already_installed` 覆盖、无需再装的引用。
    pub satisfied: Vec<ModRef>,
    /// 无法解析的引用:registry 里没有对应 provider,或该项目没有兼容当前
    /// `mc_version` + `loader` 的版本/主文件。
    pub unresolved: Vec<ModRef>,
    /// 声明为 `incompatible` 的依赖(冲突),交由上层提示用户。
    pub incompatible: Vec<ModRef>,
}

/// 在 `registry` 上解析 `roots` 的依赖闭包。
///
/// - `roots`:要安装的项目引用(每个携带其 provider)。
/// - `mc_version` / `loader`:目标实例的游戏版本与加载器(如 `"1.20.1"` / `"fabric"`),
///   用于过滤 `list_versions` 并挑选最佳版本。
/// - `already_installed`:已安装项目的键集合,键形如 [`ModRef::key`](ModRef::key)
///   (`<provider>:<project_id>`);命中的引用直接判为已满足、不再递归。
///
/// 返回 [`DepResolution`] 的四张清单。任一 provider 的网络/解析错误会以 `?` 冒泡为
/// [`crate::error::CoreError`];「项目存在但无兼容版本」不是错误,记入 `unresolved`。
pub async fn resolve_dependencies(
    registry: &ProviderRegistry,
    roots: &[ModRef],
    mc_version: &str,
    loader: &str,
    already_installed: &HashSet<String>,
) -> Result<DepResolution> {
    let mut out = DepResolution::default();

    // 已处理(无论结果落在哪张清单)的键,避免重复请求 / 重复入清单。
    let mut visited: HashSet<String> = HashSet::new();
    // 队列元素携带深度;超过 MAX_DEPTH 即停止继续展开(护栏)。
    let mut queue: VecDeque<(ModRef, usize)> = VecDeque::new();

    for r in roots {
        // 入队前就标记 visited,保证同一根/依赖不会被多条路径重复排队。
        if visited.insert(r.key()) {
            queue.push_back((r.clone(), 0));
        }
    }

    while let Some((mod_ref, depth)) = queue.pop_front() {
        // 已安装 → 已满足,不解析、不递归。
        if already_installed.contains(&mod_ref.key()) {
            out.satisfied.push(mod_ref);
            continue;
        }

        // registry 缺少该 provider → 无法解析。
        let provider = match registry.get(mod_ref.provider) {
            Some(p) => p,
            None => {
                out.unresolved.push(mod_ref);
                continue;
            }
        };

        let versions = provider
            .list_versions(&mod_ref.project_id, Some(mc_version), Some(loader))
            .await?;

        // 挑最佳兼容版本;无任何可用版本/主文件 → 无法解析。
        let picked = match pick_best_version(&versions, mc_version, loader) {
            Some(v) => v,
            None => {
                out.unresolved.push(mod_ref);
                continue;
            }
        };
        let file = match picked.primary_file() {
            Some(f) => f.clone(),
            None => {
                out.unresolved.push(mod_ref);
                continue;
            }
        };

        out.to_install.push(ResolvedFile {
            provider: mod_ref.provider,
            project_id: mod_ref.project_id.clone(),
            version_id: picked.id.clone(),
            file,
            project_name: None,
            project_slug: None,
            authors: Vec::new(),
        });

        // 到达深度护栏:登记本项目自身,但不再展开它的依赖(防环/防失控)。
        if depth >= MAX_DEPTH {
            continue;
        }

        for dep in &picked.dependencies {
            let dep_project = match dep.project_id.as_deref() {
                // 只能按 project_id 继续游走;纯 version_id 依赖(无 project_id)无法在
                // 本层定位项目,跳过(上层若需要可单独按 version 取文件)。
                Some(id) if !id.is_empty() => id,
                _ => continue,
            };
            // 依赖与父项目同源(Modrinth 依赖另一个 Modrinth 项目,CF 同理)。
            let dep_ref = ModRef::new(mod_ref.provider, dep_project);

            match dep.dependency_type.as_str() {
                "required" => {
                    if visited.insert(dep_ref.key()) {
                        queue.push_back((dep_ref, depth + 1));
                    }
                }
                "incompatible" => {
                    // 冲突也去重一次,避免同一冲突被多个父项目重复登记。
                    if visited.insert(dep_ref.key()) {
                        out.incompatible.push(dep_ref);
                    }
                }
                // optional / embedded / 其它劝告性依赖:不自动安装。
                _ => {}
            }
        }
    }

    Ok(out)
}

/// 从一个项目的版本列表里挑「最佳」版本。
///
/// 偏好序(对齐设计文档 §3):
/// 1. `game_versions` 含 `mc_version` **且** `loaders` 含 `loader` 的第一个版本(完全兼容);
/// 2. 仅 `game_versions` 含 `mc_version` 的第一个版本(游戏版本对,loader 信息可能缺失);
/// 3. 仅 `loaders` 含 `loader` 的第一个版本;
/// 4. 退化:列表里的第一个版本(`list_versions` 已按版本过滤,首项即最新兼容项的代理)。
///
/// `ProjectVersion` 无发布日期,故以「列表首个命中项」近似「最新」——provider 的版本
/// 列表本就按时间倒序返回。空列表返回 `None`。
fn pick_best_version<'a>(
    versions: &'a [ProjectVersion],
    mc_version: &str,
    loader: &str,
) -> Option<&'a ProjectVersion> {
    let mc_ok = |v: &ProjectVersion| v.game_versions.iter().any(|g| g == mc_version);
    // Quilt 实例接受 fabric 版本;其余 loader 即自身。
    let accepted = super::accepted_loaders(loader);
    let loader_ok = |v: &ProjectVersion| {
        v.loaders
            .iter()
            .any(|l| accepted.iter().any(|a| a.eq_ignore_ascii_case(l)))
    };

    versions
        .iter()
        .find(|v| mc_ok(v) && loader_ok(v))
        .or_else(|| versions.iter().find(|v| mc_ok(v)))
        .or_else(|| versions.iter().find(|v| loader_ok(v)))
        .or_else(|| versions.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ProjectSideSupport;
    use crate::error::CoreError;
    use crate::modplatform::provider::ResourceProvider;
    use crate::modplatform::{
        Dependency, HashAlgo, ProviderCaps, SearchHit, SearchQuery, VersionFile,
    };
    use futures::future::BoxFuture;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// 内存版 provider:只实现 `caps` 与 `list_versions`(依赖解析唯一用到的两个方法)。
    /// 其余 trait 方法返回空/错误即可,因为解析器不调用它们。
    struct FakeProvider {
        caps: ProviderCaps,
        /// project_id -> 该项目的版本列表(已按"最新在前"排好)。
        versions: HashMap<String, Vec<ProjectVersion>>,
    }

    impl FakeProvider {
        fn new(id: ProviderId, versions: HashMap<String, Vec<ProjectVersion>>) -> Self {
            Self {
                caps: ProviderCaps {
                    id,
                    readable_name: "fake",
                    hash_algos: &[HashAlgo::Sha1],
                    needs_api_key: false,
                },
                versions,
            }
        }
    }

    impl ResourceProvider for FakeProvider {
        fn caps(&self) -> &ProviderCaps {
            &self.caps
        }

        fn search<'a>(&'a self, _q: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_project<'a>(&'a self, _project_id: &'a str) -> BoxFuture<'a, Result<SearchHit>> {
            Box::pin(async move { Err(CoreError::other("FakeProvider::get_project unused")) })
        }

        fn get_projects<'a>(
            &'a self,
            _project_ids: &'a [String],
        ) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn list_versions<'a>(
            &'a self,
            project_id: &'a str,
            _game_version: Option<&'a str>,
            _loader: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<ProjectVersion>>> {
            let result = self.versions.get(project_id).cloned().unwrap_or_default();
            Box::pin(async move { Ok(result) })
        }

        fn resolve_by_hashes<'a>(
            &'a self,
            _algo: HashAlgo,
            _hashes: &'a [String],
        ) -> BoxFuture<'a, Result<Vec<Option<ResolvedFile>>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_files_bulk<'a>(
            &'a self,
            _refs: &'a [(String, String)],
        ) -> BoxFuture<'a, Result<Vec<ResolvedFile>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }
    }

    /// 造一个兼容 `1.20.1` + `fabric`、带可选 `deps` 的版本,主文件 url 取自 `id`。
    fn version_with_deps(id: &str, deps: Vec<Dependency>) -> ProjectVersion {
        ProjectVersion {
            id: format!("{id}-v1"),
            name: format!("{id} 1.0"),
            version_number: "1.0".into(),
            game_versions: vec!["1.20.1".into()],
            loaders: vec!["fabric".into()],
            files: vec![VersionFile {
                url: format!("https://example.invalid/{id}.jar"),
                filename: format!("{id}.jar"),
                primary: true,
                ..Default::default()
            }],
            dependencies: deps,
            client_side: ProjectSideSupport::Unknown,
            server_side: ProjectSideSupport::Unknown,
        }
    }

    fn required_on(project_id: &str) -> Dependency {
        Dependency {
            project_id: Some(project_id.into()),
            version_id: None,
            dependency_type: "required".into(),
        }
    }

    fn registry_with(versions: HashMap<String, Vec<ProjectVersion>>) -> ProviderRegistry {
        let provider: Arc<dyn ResourceProvider> =
            Arc::new(FakeProvider::new(ProviderId::Modrinth, versions));
        ProviderRegistry::new().with(provider)
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        futures::executor::block_on(f)
    }

    #[test]
    fn pick_best_prefers_mc_and_loader_match() {
        let versions = vec![
            // 只匹配 loader,不匹配 mc。
            ProjectVersion {
                id: "wrong-mc".into(),
                name: "x".into(),
                version_number: "1".into(),
                game_versions: vec!["1.19.2".into()],
                loaders: vec!["fabric".into()],
                files: vec![],
                dependencies: vec![],
                client_side: ProjectSideSupport::Unknown,
                server_side: ProjectSideSupport::Unknown,
            },
            // 完全匹配。
            ProjectVersion {
                id: "perfect".into(),
                name: "y".into(),
                version_number: "2".into(),
                game_versions: vec!["1.20.1".into()],
                loaders: vec!["fabric".into()],
                files: vec![],
                dependencies: vec![],
                client_side: ProjectSideSupport::Unknown,
                server_side: ProjectSideSupport::Unknown,
            },
        ];
        let best = pick_best_version(&versions, "1.20.1", "fabric").unwrap();
        assert_eq!(best.id, "perfect");
    }

    #[test]
    fn pick_best_falls_back_to_first_when_none_match() {
        let versions = vec![ProjectVersion {
            id: "only".into(),
            name: "x".into(),
            version_number: "1".into(),
            game_versions: vec!["1.7.10".into()],
            loaders: vec!["forge".into()],
            files: vec![],
            dependencies: vec![],
            client_side: ProjectSideSupport::Unknown,
            server_side: ProjectSideSupport::Unknown,
        }];
        // 既不匹配 mc 也不匹配 loader → 退化到首项。
        let best = pick_best_version(&versions, "1.20.1", "fabric").unwrap();
        assert_eq!(best.id, "only");
        assert!(pick_best_version(&[], "1.20.1", "fabric").is_none());
    }

    #[test]
    fn root_with_required_dep_installs_both() {
        // root 依赖 lib;两者都应进 to_install。
        let mut versions = HashMap::new();
        versions.insert("root".to_string(), vec![version_with_deps("root", vec![required_on("lib")])]);
        versions.insert("lib".to_string(), vec![version_with_deps("lib", vec![])]);
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "root")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        let ids: HashSet<&str> = res.to_install.iter().map(|r| r.project_id.as_str()).collect();
        assert!(ids.contains("root"), "root should be installed");
        assert!(ids.contains("lib"), "required dep should be installed");
        assert_eq!(res.to_install.len(), 2);
        assert!(res.unresolved.is_empty());
        assert!(res.incompatible.is_empty());
        assert!(res.satisfied.is_empty());
        // 主文件 url 正确传递。
        let lib = res.to_install.iter().find(|r| r.project_id == "lib").unwrap();
        assert_eq!(lib.file.url, "https://example.invalid/lib.jar");
        assert_eq!(lib.version_id, "lib-v1");
    }

    #[test]
    fn incompatible_dep_lands_in_incompatible() {
        let mut versions = HashMap::new();
        let root = version_with_deps(
            "root",
            vec![Dependency {
                project_id: Some("badmod".into()),
                version_id: None,
                dependency_type: "incompatible".into(),
            }],
        );
        versions.insert("root".to_string(), vec![root]);
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "root")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        assert_eq!(res.to_install.len(), 1); // 只有 root
        assert_eq!(res.incompatible.len(), 1);
        assert_eq!(res.incompatible[0].project_id, "badmod");
        // 冲突项不应被请求/安装(FakeProvider 也没有它的版本)。
        assert!(res.to_install.iter().all(|r| r.project_id != "badmod"));
    }

    #[test]
    fn already_installed_root_is_satisfied_not_installed() {
        let mut versions = HashMap::new();
        versions.insert("root".to_string(), vec![version_with_deps("root", vec![])]);
        let registry = registry_with(versions);

        let mut installed = HashSet::new();
        installed.insert(ModRef::new(ProviderId::Modrinth, "root").key());

        let roots = [ModRef::new(ProviderId::Modrinth, "root")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &installed,
        ))
        .unwrap();

        assert!(res.to_install.is_empty(), "already-installed root must not reinstall");
        assert_eq!(res.satisfied.len(), 1);
        assert_eq!(res.satisfied[0].project_id, "root");
    }

    #[test]
    fn already_installed_dep_short_circuits_recursion() {
        // root -> lib(required),但 lib 已装 → lib 进 satisfied、不递归其依赖。
        let mut versions = HashMap::new();
        versions.insert("root".to_string(), vec![version_with_deps("root", vec![required_on("lib")])]);
        versions.insert(
            "lib".to_string(),
            vec![version_with_deps("lib", vec![required_on("deep")])],
        );
        versions.insert("deep".to_string(), vec![version_with_deps("deep", vec![])]);
        let registry = registry_with(versions);

        let mut installed = HashSet::new();
        installed.insert(ModRef::new(ProviderId::Modrinth, "lib").key());

        let roots = [ModRef::new(ProviderId::Modrinth, "root")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &installed,
        ))
        .unwrap();

        let ids: HashSet<&str> = res.to_install.iter().map(|r| r.project_id.as_str()).collect();
        assert!(ids.contains("root"));
        // lib 已装 → 不装、且其依赖 deep 不被拉取。
        assert!(!ids.contains("lib"));
        assert!(!ids.contains("deep"));
        assert!(res.satisfied.iter().any(|m| m.project_id == "lib"));
    }

    #[test]
    fn missing_provider_marks_unresolved() {
        // 引用一个 registry 里不存在的 provider(CurseForge 未注册)。
        let versions = HashMap::new();
        let registry = registry_with(versions); // 只注册了 Modrinth

        let roots = [ModRef::new(ProviderId::CurseForge, "whatever")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        assert!(res.to_install.is_empty());
        assert_eq!(res.unresolved.len(), 1);
        assert_eq!(res.unresolved[0].provider, ProviderId::CurseForge);
    }

    #[test]
    fn project_without_compatible_version_is_unresolved() {
        // 项目存在但没有任何版本 → unresolved(不是错误)。
        let mut versions = HashMap::new();
        versions.insert("empty".to_string(), Vec::new());
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "empty")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        assert!(res.to_install.is_empty());
        assert_eq!(res.unresolved.len(), 1);
        assert_eq!(res.unresolved[0].project_id, "empty");
    }

    #[test]
    fn cycle_terminates_via_visited_set() {
        // a -> b -> a(环)。visited 去重保证每个只解析一次、循环终止。
        let mut versions = HashMap::new();
        versions.insert("a".to_string(), vec![version_with_deps("a", vec![required_on("b")])]);
        versions.insert("b".to_string(), vec![version_with_deps("b", vec![required_on("a")])]);
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "a")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        let ids: HashSet<&str> = res.to_install.iter().map(|r| r.project_id.as_str()).collect();
        assert!(ids.contains("a"));
        assert!(ids.contains("b"));
        assert_eq!(res.to_install.len(), 2, "each node resolved exactly once despite the cycle");
    }

    #[test]
    fn self_loop_terminates() {
        // a -> a(自环)。
        let mut versions = HashMap::new();
        versions.insert("a".to_string(), vec![version_with_deps("a", vec![required_on("a")])]);
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "a")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        assert_eq!(res.to_install.len(), 1);
        assert_eq!(res.to_install[0].project_id, "a");
    }

    #[test]
    fn duplicate_roots_dedup() {
        // 同一个根给两次,只应解析/安装一次。
        let mut versions = HashMap::new();
        versions.insert("root".to_string(), vec![version_with_deps("root", vec![])]);
        let registry = registry_with(versions);

        let roots = [
            ModRef::new(ProviderId::Modrinth, "root"),
            ModRef::new(ProviderId::Modrinth, "root"),
        ];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        assert_eq!(res.to_install.len(), 1);
    }

    #[test]
    fn optional_dep_is_skipped() {
        let mut versions = HashMap::new();
        let root = version_with_deps(
            "root",
            vec![Dependency {
                project_id: Some("nicetohave".into()),
                version_id: None,
                dependency_type: "optional".into(),
            }],
        );
        versions.insert("root".to_string(), vec![root]);
        versions.insert("nicetohave".to_string(), vec![version_with_deps("nicetohave", vec![])]);
        let registry = registry_with(versions);

        let roots = [ModRef::new(ProviderId::Modrinth, "root")];
        let res = run(resolve_dependencies(
            &registry,
            &roots,
            "1.20.1",
            "fabric",
            &HashSet::new(),
        ))
        .unwrap();

        // optional 不自动安装。
        assert_eq!(res.to_install.len(), 1);
        assert_eq!(res.to_install[0].project_id, "root");
        assert!(res.incompatible.is_empty());
    }

    #[test]
    fn modref_key_distinguishes_providers() {
        let m = ModRef::new(ProviderId::Modrinth, "abc");
        let c = ModRef::new(ProviderId::CurseForge, "abc");
        assert_ne!(m.key(), c.key());
        assert_eq!(m.key(), "modrinth:abc");
        assert_eq!(c.key(), "curseforge:abc");
    }
}
