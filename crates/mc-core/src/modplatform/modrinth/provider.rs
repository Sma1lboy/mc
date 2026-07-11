use super::*;

// ============================ ResourceProvider 适配 ============================

use std::collections::HashMap;

use futures::future::{try_join_all, BoxFuture};

use crate::modplatform::provider::ResourceProvider;
use crate::modplatform::{HashAlgo, ProviderCaps, ProviderId, ResolvedFile};

/// Modrinth 支持反查的哈希算法,按偏好序(sha512 优先,sha1 兜底)。
/// `&'static [HashAlgo]` 需要一个 `'static` 数组,故声明为 const。
const MODRINTH_HASH_ALGOS: &[HashAlgo] = &[HashAlgo::Sha512, HashAlgo::Sha1];

/// Modrinth 的能力声明(`const`,无运行时输入)。
const MODRINTH_CAPS: ProviderCaps = ProviderCaps {
    id: ProviderId::Modrinth,
    readable_name: "Modrinth",
    hash_algos: MODRINTH_HASH_ALGOS,
    needs_api_key: false,
};

/// 把统一 [`SearchQuery`] 适配到 [`ModrinthApi`] 的 [`ResourceProvider`] 实现。
/// 持有一个 [`ModrinthApi`](内含配好 UA 的 `reqwest::Client`)。
#[derive(Debug, Clone, Default)]
pub struct ModrinthProvider {
    api: ModrinthApi,
}

impl ModrinthProvider {
    /// 默认 base url(`https://api.modrinth.com/v2`)的 provider。
    pub fn new() -> Self {
        Self { api: ModrinthApi::new() }
    }

    /// 用自定义 base url 构造(测试 / 镜像)。
    pub fn with_base(base: impl Into<String>) -> Self {
        Self { api: ModrinthApi::with_base(base) }
    }
}

/// 把统一 [`HashAlgo`] 映射到 Modrinth `/version_files` 的 `algorithm` 字符串。
/// Modrinth 只支持 sha1 / sha512;其余算法不可反查。
pub(crate) fn modrinth_algo_str(algo: HashAlgo) -> Result<&'static str> {
    match algo {
        HashAlgo::Sha512 => Ok("sha512"),
        HashAlgo::Sha1 => Ok("sha1"),
        HashAlgo::Md5 | HashAlgo::Murmur2 => {
            Err(CoreError::other("unsupported hash algo for Modrinth"))
        }
    }
}

impl ResourceProvider for ModrinthProvider {
    fn caps(&self) -> &ProviderCaps {
        &MODRINTH_CAPS
    }

    fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
        Box::pin(async move { self.api.search_query(q).await })
    }

    fn get_project<'a>(&'a self, project_id: &'a str) -> BoxFuture<'a, Result<SearchHit>> {
        Box::pin(async move { self.api.get_project(project_id).await })
    }

    fn get_projects<'a>(
        &'a self,
        project_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
        Box::pin(async move { self.api.get_projects(project_ids).await })
    }

    fn list_versions<'a>(
        &'a self,
        project_id: &'a str,
        game_version: Option<&'a str>,
        loader: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<ProjectVersion>>> {
        Box::pin(async move { self.api.get_versions(project_id, game_version, loader).await })
    }

    fn resolve_by_hashes<'a>(
        &'a self,
        algo: HashAlgo,
        hashes: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Option<ResolvedFile>>>> {
        Box::pin(async move {
            let algorithm = modrinth_algo_str(algo)?;
            let by_hash: HashMap<String, RawVersion> =
                self.api.raw_versions_from_hashes(hashes, algorithm).await?;

            // 输出严格与输入 `hashes` 对齐:逐个查表,命中后再在版本的文件里按算法
            // 找到哈希恰好相等的那个文件(一个版本可能挂多个文件)。
            let out = hashes
                .iter()
                .map(|h| {
                    let version = by_hash.get(h)?;
                    let file = find_file_by_hash(version, algo, h)?;
                    Some(ResolvedFile {
                        provider: ProviderId::Modrinth,
                        project_id: version.project_id.clone(),
                        version_id: version.id.clone(),
                        file,
                        project_name: None,
                        project_slug: None,
                        authors: Vec::new(),
                    })
                })
                .collect();
            Ok(out)
        })
    }

    fn get_files_bulk<'a>(
        &'a self,
        refs: &'a [(String, String)],
    ) -> BoxFuture<'a, Result<Vec<ResolvedFile>>> {
        Box::pin(async move {
            // Modrinth 无批量 version 端点,逐个 `/version/{id}` 并发取。`refs` 是
            // (project_id, version_id);项目 id 直接用作 ResolvedFile.project_id。
            let futures = refs.iter().map(|(project_id, version_id)| async move {
                let version = self.api.get_version(version_id).await?;
                // 主文件即下载目标;没有文件的版本视为无法解析。
                let file = version.primary_file().cloned().ok_or_else(|| {
                    CoreError::other(format!("Modrinth version {version_id} has no files"))
                })?;
                Ok::<ResolvedFile, CoreError>(ResolvedFile {
                    provider: ProviderId::Modrinth,
                    project_id: project_id.clone(),
                    version_id: version.id.clone(),
                    file,
                    project_name: None,
                    project_slug: None,
                    authors: Vec::new(),
                })
            });
            try_join_all(futures).await
        })
    }
}
