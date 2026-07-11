//! 内容平台 Provider 抽象:搜索 / 详情 / 版本 / **按哈希反查** / 批量取文件,各平台一个
//! 可插拔实现。导入(id→URL via [`ResourceProvider::get_files_bulk`])、导出(hash→引用 via
//! [`ResourceProvider::resolve_by_hashes`])、浏览三者共用同一抽象。
//!
//! 不引 `async-trait`(对齐 [`crate::modplatform`] 既有"不加依赖"的约定):trait 方法返回
//! [`futures::future::BoxFuture`],各实现内 `Box::pin(async move { … })`。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::StreamExt;

use crate::error::{CoreError, Result};

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

/// 同时在途的 provider 请求上限:并发省时,又不至于猛打 provider 触发限流。多路 agent 搜索面
/// (base 搜整合包 / customization 搜 mod)此前各自复制一份同名常量,现集中于此。
pub const PROVIDER_FANOUT: usize = 8;

/// [`ProviderRegistry::search_concurrent`] 的一条输出:命中来自哪个 provider、由入参 `queries`
/// 里第几条查询 surfaced、命中本身。已去重、按输入(query-major, provider)序、并按
/// [`DedupCapPolicy`] 截断。
#[derive(Debug, Clone)]
pub struct SearchMatch {
    /// 产出该命中的平台。
    pub provider: ProviderId,
    /// 命中所属查询在入参 `queries` 中的下标(调用方据此回填自己的 `matched_query`)。
    pub query_index: usize,
    /// 命中本身。
    pub hit: SearchHit,
}

/// 并发搜索 fan-out 到哪些 provider。
#[derive(Debug, Clone)]
pub enum ProviderTargets {
    /// 所有已注册 provider,按 [`ProviderRegistry::all`] 的迭代序(对未变更的注册表稳定)。
    All,
    /// 恰好这些 provider,按给定序;每个都必须已注册(缺任一则整次搜索报错)。
    Only(Vec<ProviderId>),
}

/// 并发搜索的「去重 + 截断」策略。两处调用面各自的行为都能由它表达:
/// - base 搜整合包:仅总量上限(`per_query_cap = None`),且只搜 Modrinth。
/// - customization 搜 mod:每查询上限 + 总量上限,跨全部 provider。
#[derive(Debug, Clone)]
pub struct DedupCapPolicy {
    /// fan-out 到哪些 provider。
    pub providers: ProviderTargets,
    /// 每条查询「计入结果」的去重命中数上限;`None` = 不限。达到上限只中断该查询**当前 provider**
    /// 的命中流(与旧内联 `break` 逐字一致:后续 provider 仍会被消费、其首个未去重命中仍可入选,
    /// 故多 provider 下这是软上限)。
    pub per_query_cap: Option<usize>,
    /// 跨所有 query × provider 的结果总量上限;一旦达到立即返回已收集结果。
    pub total_cap: usize,
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
    ///
    /// 等价于 `with_defaults_keyed(None)`:CurseForge key 仅从环境解析(保持旧行为)。
    pub fn with_defaults() -> Self {
        Self::with_defaults_keyed(None)
    }

    /// 同 [`Self::with_defaults`],但允许传入一个显式 CurseForge key(通常来自用户设置)。
    /// key 经 [`resolve_cf_api_key`] 解析(settings → 编译期 baked → 环境);解析出 key 才
    /// 注册 CurseForge,否则只注册 Modrinth。
    pub fn with_defaults_keyed(cf_key: Option<String>) -> Self {
        let mut reg = Self::new().with(Arc::new(super::modrinth::ModrinthProvider::new()));
        if let Some(key) = resolve_cf_api_key(cf_key.as_deref()) {
            if let Some(cf) = super::curseforge::CurseForgeProvider::from_key(key) {
                reg = reg.with(Arc::new(cf));
            }
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

    /// 多 provider × 多 query 的并发搜索,输出**已去重、按输入(query-major, provider)序、并按
    /// `policy` 截断**的结果。集中了各 agent 搜索面此前逐字重复的编排:
    ///
    /// 1. **有界有序并发**:把每个 `(query × provider)` 搜索排成 query 大序、provider 小序,以
    ///    `buffered(fanout)` 跑——结果按提交序产出(与完成序无关),故下面的去重 + 截断确定且与
    ///    「逐 `(query, provider)` 顺序遍历」逐字一致。
    /// 2. **错误的截断丢弃语义**:结果用 `Vec<Result<…>>` 收集(**非** `try_collect`),故被 cap
    ///    跳过、从未被消费的 query/provider,其错误被丢弃;而在 cap 之前**实际消费**到的第一个错误
    ///    仍照常经 `?` 传播。
    /// 3. **唯一的 `(ProviderId, hit.id)` 去重键**:等价于旧的 `format!("{provider:?}:{id}")` 字符串
    ///    键(provider Debug 名不含 `:`,首个 `:` 恒分隔平台与 id,故两者去重结果逐位相同)。
    /// 4. **per-query / total cap** 在有序结果上重放(见 [`DedupCapPolicy`] 对软上限的说明)。
    pub async fn search_concurrent(
        &self,
        queries: &[SearchQuery],
        fanout: usize,
        policy: DedupCapPolicy,
    ) -> Result<Vec<SearchMatch>> {
        // Freeze the provider fan-out set once, in the exact requested order, and reuse it for both
        // future submission and the replay stride below.
        let providers: Vec<Arc<dyn ResourceProvider>> = match &policy.providers {
            ProviderTargets::All => self.all().cloned().collect(),
            ProviderTargets::Only(ids) => {
                let mut chosen = Vec::with_capacity(ids.len());
                for id in ids {
                    let provider = self.get(*id).ok_or_else(|| {
                        CoreError::other(format!("provider {id:?} is not registered"))
                    })?;
                    chosen.push(provider);
                }
                chosen
            }
        };

        // Fan out every (query × provider) search concurrently in query-major / provider order.
        // `buffered` yields in that submission order regardless of completion, so the dedup + caps
        // replayed below are deterministic. Collect into `Vec<Result<…>>` (NOT `try_collect`) so a
        // search the caps would skip cannot surface its error — the first error we actually consume
        // still propagates via `?`.
        let mut futs = Vec::with_capacity(queries.len().saturating_mul(providers.len()));
        for query in queries {
            for provider in &providers {
                futs.push(async move { (provider.id(), provider.search(query).await) });
            }
        }
        let flat: Vec<(ProviderId, Result<Vec<SearchHit>>)> =
            futures::stream::iter(futs).buffered(fanout).collect().await;

        // Replay the ordered results into the deduped, capped output.
        let mut out: Vec<SearchMatch> = Vec::new();
        let mut seen: HashSet<(ProviderId, String)> = HashSet::new();
        let mut flat_iter = flat.into_iter();
        for (query_index, _query) in queries.iter().enumerate() {
            let mut query_results = 0usize;
            for _ in 0..providers.len() {
                let (provider_id, hits_result) = flat_iter
                    .next()
                    .expect("flat holds exactly queries.len() * providers.len() entries");
                for hit in hits_result? {
                    if !seen.insert((provider_id, hit.id.clone())) {
                        continue;
                    }
                    out.push(SearchMatch {
                        provider: provider_id,
                        query_index,
                        hit,
                    });
                    query_results += 1;
                    if out.len() >= policy.total_cap {
                        return Ok(out);
                    }
                    if let Some(cap) = policy.per_query_cap {
                        if query_results >= cap {
                            break;
                        }
                    }
                }
            }
        }

        Ok(out)
    }
}

/// 解析最终生效的 CurseForge API key,按优先级:
/// 1. `settings_key`(用户在设置里填的,去空白后非空)——最高优先,Prism 风格自带 key。
/// 2. `option_env!("MC_CF_API_KEY")`——编译期 baked 进二进制的发行 key(若构建时配了)。
/// 3. `std::env::var("MC_CF_API_KEY")`——运行期环境变量(本地开发 / CI)。
/// 4. 都没有 → `None`(上层据此不注册 CurseForge)。
///
/// 每一层都做 trim + 空串守卫;返回的 key 已去空白。**secret,勿打日志。**
pub fn resolve_cf_api_key(settings_key: Option<&str>) -> Option<String> {
    fn non_empty(s: &str) -> Option<String> {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    }

    settings_key
        .and_then(non_empty)
        .or_else(|| option_env!("MC_CF_API_KEY").and_then(non_empty))
        .or_else(|| std::env::var("MC_CF_API_KEY").ok().as_deref().and_then(non_empty))
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
mod tests;
