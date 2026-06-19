//! 内容平台 Provider 抽象:搜索 / 详情 / 版本 / **按哈希反查** / 批量取文件,各平台一个
//! 可插拔实现。导入(id→URL via [`ResourceProvider::get_files_bulk`])、导出(hash→引用 via
//! [`ResourceProvider::resolve_by_hashes`])、浏览三者共用同一抽象。
//!
//! 不引 `async-trait`(对齐 [`crate::modplatform`] 既有"不加依赖"的约定):trait 方法返回
//! [`futures::future::BoxFuture`],各实现内 `Box::pin(async move { … })`。

use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;

use crate::error::Result;

use super::{
    HashAlgo, ProjectVersion, ProviderCaps, ProviderId, ResolvedFile, SearchHit, SearchQuery,
};

/// 一个内容平台后端。对象安全(`Arc<dyn ResourceProvider>` 可入注册表)。
pub trait ResourceProvider: Send + Sync {
    /// 能力声明(id / 反查算法 / 是否需要 key)。
    fn caps(&self) -> &ProviderCaps;

    /// 平台标识(默认取自 [`Self::caps`])。
    fn id(&self) -> ProviderId {
        self.caps().id
    }

    /// 搜索项目。
    fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<SearchHit>>>;

    /// 取单个项目元信息。
    fn get_project<'a>(&'a self, project_id: &'a str) -> BoxFuture<'a, Result<SearchHit>>;

    /// 批量取项目元信息(对齐 Prism `getProjects`)。顺序不保证与输入一致。
    fn get_projects<'a>(
        &'a self,
        project_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<SearchHit>>>;

    /// 列出某项目的版本,可按游戏版本 / loader 过滤。
    fn list_versions<'a>(
        &'a self,
        project_id: &'a str,
        game_version: Option<&'a str>,
        loader: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<ProjectVersion>>>;

    /// 批量哈希 → 文件(返回下标与 `hashes` 对齐;无匹配为 `None`)。`algo` 须在
    /// [`ProviderCaps::hash_algos`] 内。Modrinth=POST `/version_files`,CurseForge=POST
    /// `/fingerprints`(murmur2)。导出反查与导入去重的命脉。
    fn resolve_by_hashes<'a>(
        &'a self,
        algo: HashAlgo,
        hashes: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Option<ResolvedFile>>>>;

    /// 批量按 `(project_id, version_id)` 取文件——导入把 manifest 里的 id 变成下载 URL
    /// (CurseForge POST `/mods/files`;Modrinth 逐 `/version/{id}`)。
    fn get_files_bulk<'a>(
        &'a self,
        refs: &'a [(String, String)],
    ) -> BoxFuture<'a, Result<Vec<ResolvedFile>>>;
}

/// Provider 注册表:按平台 id 或按下载 host 选取。导入与导出共用同一份注册表。
#[derive(Default, Clone)]
pub struct ProviderRegistry {
    by_id: HashMap<ProviderId, Arc<dyn ResourceProvider>>,
}

impl ProviderRegistry {
    /// 空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 内建默认注册表:**总是**注册 Modrinth(无需 key);若环境里配了 CurseForge API key
    /// (`MC_CF_API_KEY`),再注册 CurseForge —— 无 key 就不注册(而非塞个会 401 的)。
    ///
    /// 整合包导入据此让 curseforge / mcbbs 的 `resolve()` 把 manifest 里的 id 变成下载 URL:
    /// 没配 key 时 CurseForge provider 缺席,resolve 会明确报「需配置 API key」而非静默失败。
    pub fn with_defaults() -> Self {
        let mut reg = Self::new().with(Arc::new(super::modrinth::ModrinthProvider::new()));
        if let Some(cf) = super::curseforge::CurseForgeProvider::from_env() {
            reg = reg.with(Arc::new(cf));
        }
        reg
    }

    /// 注册一个 provider(链式)。
    pub fn with(mut self, provider: Arc<dyn ResourceProvider>) -> Self {
        self.by_id.insert(provider.id(), provider);
        self
    }

    /// 按平台标识取。
    pub fn get(&self, id: ProviderId) -> Option<Arc<dyn ResourceProvider>> {
        self.by_id.get(&id).cloned()
    }

    /// 按下载 URL 的 host 反查所属平台(导出免费 resolve / 导入去重用)。
    pub fn for_host(&self, host: &str) -> Option<Arc<dyn ResourceProvider>> {
        host_provider(host).and_then(|id| self.get(id))
    }

    /// 遍历已注册的 provider。
    pub fn all(&self) -> impl Iterator<Item = &Arc<dyn ResourceProvider>> {
        self.by_id.values()
    }
}

/// 把一个 host 映射到所属平台(纯函数,可单测):
/// `*.modrinth.com` → Modrinth;`*.forgecdn.net` / `*.curseforge.com` → CurseForge。
pub fn host_provider(host: &str) -> Option<ProviderId> {
    let h = host.to_ascii_lowercase();
    if h.ends_with("modrinth.com") {
        Some(ProviderId::Modrinth)
    } else if h.ends_with("forgecdn.net") || h.ends_with("curseforge.com") {
        Some(ProviderId::CurseForge)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_routing_maps_known_cdns() {
        assert_eq!(host_provider("cdn.modrinth.com"), Some(ProviderId::Modrinth));
        assert_eq!(host_provider("api.modrinth.com"), Some(ProviderId::Modrinth));
        assert_eq!(host_provider("edge.forgecdn.net"), Some(ProviderId::CurseForge));
        assert_eq!(host_provider("mediafilez.forgecdn.net"), Some(ProviderId::CurseForge));
        assert_eq!(host_provider("api.curseforge.com"), Some(ProviderId::CurseForge));
        assert_eq!(host_provider("example.com"), None);
    }
}
