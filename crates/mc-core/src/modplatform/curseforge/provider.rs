use super::*;

// ============================ Provider 实现 ============================

/// CurseForge 的 [`ResourceProvider`] 实现,内含一个 [`FlameApi`]。
pub struct CurseForgeProvider {
    api: FlameApi,
}

/// CurseForge 能力声明:需要 API key,反查算法仅 murmur2(CF 指纹反查端点)。
static CURSEFORGE_CAPS: ProviderCaps = ProviderCaps {
    id: ProviderId::CurseForge,
    readable_name: "CurseForge",
    hash_algos: &[HashAlgo::Murmur2],
    needs_api_key: true,
};

impl CurseForgeProvider {
    /// 用一个已配置好的 [`FlameApi`] 构造。
    pub fn new(api: FlameApi) -> Self {
        Self { api }
    }

    /// 便捷:从 env 构造(无 key 则 `None`,上层据此决定是否注册)。
    pub fn from_env() -> Option<Self> {
        FlameApi::from_env().map(Self::new)
    }

    /// 便捷:从显式 key 构造(空/全空白则 `None`)。用户设置里填的 CurseForge key 走这条路。
    pub fn from_key(key: impl Into<String>) -> Option<Self> {
        FlameApi::from_key(key).map(Self::new)
    }

    /// 取底层 [`FlameApi`](诊断/复用用)。
    pub fn api(&self) -> &FlameApi {
        &self.api
    }
}

use crate::modplatform::provider::ResourceProvider;
use futures::future::BoxFuture;

impl ResourceProvider for CurseForgeProvider {
    fn caps(&self) -> &ProviderCaps {
        &CURSEFORGE_CAPS
    }

    fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
        Box::pin(async move { self.api.search(q).await })
    }

    fn get_project<'a>(&'a self, project_id: &'a str) -> BoxFuture<'a, Result<SearchHit>> {
        Box::pin(async move {
            let id = parse_id(project_id, "CurseForge project id")?;
            let mods = self.api.get_mods(&[id]).await?;
            mods.into_iter()
                .next()
                .map(map_project)
                .ok_or_else(|| CoreError::other(format!("CurseForge project {project_id} not found")))
        })
    }

    fn get_projects<'a>(
        &'a self,
        project_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
        Box::pin(async move {
            let ids = parse_ids(project_ids, "CurseForge project id")?;
            let mods = self.api.get_mods(&ids).await?;
            Ok(mods.into_iter().map(map_project).collect())
        })
    }

    fn list_versions<'a>(
        &'a self,
        project_id: &'a str,
        game_version: Option<&'a str>,
        loader: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<ProjectVersion>>> {
        Box::pin(async move {
            let id = parse_id(project_id, "CurseForge project id")?;
            let url = format!("{}/mods/{}/files", self.api.base, id);

            // 首页即可(CF `pageSize` 上限 50);上层一般取最近若干版本。
            let mut params: Vec<(&str, String)> = vec![
                ("index", "0".to_string()),
                ("pageSize", "50".to_string()),
            ];
            if let Some(v) = game_version.filter(|s| !s.is_empty()) {
                params.push(("gameVersion", v.to_string()));
            }
            if let Some(t) = loader.and_then(loader_type_id) {
                params.push(("modLoaderType", t.to_string()));
            }

            let resp: FlameEnvelope<Vec<FlameApiFile>> = self
                .api
                .client
                .get(&url)
                .query(&params)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            Ok(resp.data.into_iter().map(map_file_to_version).collect())
        })
    }

    fn resolve_by_hashes<'a>(
        &'a self,
        algo: HashAlgo,
        hashes: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Option<ResolvedFile>>>> {
        Box::pin(async move {
            // CurseForge 只支持 murmur2 指纹反查。
            if !matches!(algo, HashAlgo::Murmur2) {
                return Err(CoreError::other(
                    "CurseForge only supports Murmur2 fingerprint lookup",
                ));
            }

            // 输入是十进制 murmur2 字符串 → u32。非法项记成 None 占位(保持与输入对齐)。
            // 同时建一张 fingerprint → 输入下标 的反查表(可能多个输入同指纹)。
            let mut fps: Vec<u32> = Vec::new();
            let mut parsed: Vec<Option<u32>> = Vec::with_capacity(hashes.len());
            for h in hashes {
                match h.trim().parse::<u32>() {
                    Ok(fp) => {
                        parsed.push(Some(fp));
                        if !fps.contains(&fp) {
                            fps.push(fp);
                        }
                    }
                    Err(_) => parsed.push(None),
                }
            }

            let mut out: Vec<Option<ResolvedFile>> = vec![None; hashes.len()];
            if fps.is_empty() {
                return Ok(out);
            }

            let matches = self.api.match_fingerprints(&fps).await?;

            // 用 fingerprint 把匹配对齐回输入下标(返回顺序不保证)。
            use std::collections::HashMap;
            let mut by_fp: HashMap<u32, &FlameApiFile> = HashMap::new();
            for m in &matches {
                if let Some(fp) = m.file.file_fingerprint {
                    // file_fingerprint 是 u64,但 CF 指纹本质是 u32;截断对齐。
                    by_fp.insert(fp as u32, &m.file);
                }
            }

            // 富化:对所有命中文件的 mod_id 批量取项目名/slug(一次请求,便宜)。
            let mod_ids: Vec<i64> = {
                let mut ids: Vec<i64> = matches
                    .iter()
                    .map(|m| m.file.mod_id)
                    .filter(|id| *id != 0)
                    .collect();
                ids.sort_unstable();
                ids.dedup();
                ids
            };
            let projects = if mod_ids.is_empty() {
                Vec::new()
            } else {
                // 取不到名字不致命:富化失败就退化为 None 名字。
                self.api.get_mods(&mod_ids).await.unwrap_or_default()
            };
            let proj_by_id: HashMap<i64, &FlameApiProject> =
                projects.iter().map(|p| (p.id, p)).collect();

            for (i, fp_opt) in parsed.into_iter().enumerate() {
                if let Some(fp) = fp_opt {
                    if let Some(file) = by_fp.get(&fp) {
                        out[i] = Some(resolved_from_file(file, proj_by_id.get(&file.mod_id).copied()));
                    }
                }
            }

            Ok(out)
        })
    }

    fn get_files_bulk<'a>(
        &'a self,
        refs: &'a [(String, String)],
    ) -> BoxFuture<'a, Result<Vec<ResolvedFile>>> {
        Box::pin(async move {
            // refs 是 (project_id, file_id) 的字符串对;我们只需 file_id 去批量取文件。
            let file_ids: Vec<i64> = refs
                .iter()
                .filter_map(|(_, fid)| fid.trim().parse::<i64>().ok())
                .collect();

            if file_ids.is_empty() {
                return Ok(Vec::new());
            }

            let files = self.api.get_files(&file_ids).await?;

            // 富化项目名/slug:对所有涉及的 mod_id 批量取一次(便宜)。
            let mod_ids: Vec<i64> = {
                let mut ids: Vec<i64> = files.iter().map(|f| f.mod_id).filter(|id| *id != 0).collect();
                ids.sort_unstable();
                ids.dedup();
                ids
            };
            let projects = if mod_ids.is_empty() {
                Vec::new()
            } else {
                self.api.get_mods(&mod_ids).await.unwrap_or_default()
            };
            use std::collections::HashMap;
            let proj_by_id: HashMap<i64, &FlameApiProject> =
                projects.iter().map(|p| (p.id, p)).collect();

            Ok(files
                .iter()
                .map(|f| resolved_from_file(f, proj_by_id.get(&f.mod_id).copied()))
                .collect())
        })
    }
}

/// 把一个 [`FlameApiFile`](+ 可选项目元信息)映射成统一 [`ResolvedFile`]。
///
/// 注意 BLOCKED 文件(`download_url == None`)依然返回一个 `ResolvedFile`,只是
/// `file.url` 为空串——调用方据"url 为空"识别 blocked 并走手动下载流。
pub(crate) fn resolved_from_file(f: &FlameApiFile, project: Option<&FlameApiProject>) -> ResolvedFile {
    ResolvedFile {
        provider: ProviderId::CurseForge,
        project_id: f.mod_id.to_string(),
        version_id: f.id.to_string(),
        file: map_version_file(f),
        project_name: project.map(|p| p.name.clone()),
        project_slug: project.map(|p| p.slug.clone()),
        authors: project
            .map(|p| p.authors.iter().map(|a| a.name.clone()).collect())
            .unwrap_or_default(),
    }
}

/// 把字符串 id 解析成 i64,失败映射成 [`CoreError::Other`](带上下文)。
pub(crate) fn parse_id(s: &str, what: &str) -> Result<i64> {
    s.trim()
        .parse::<i64>()
        .map_err(|_| CoreError::other(format!("invalid {what}: {s:?}")))
}

/// 批量把字符串 id 解析成 i64;遇到非法项直接报错(保持调用方语义明确)。
pub(crate) fn parse_ids(ids: &[String], what: &str) -> Result<Vec<i64>> {
    ids.iter().map(|s| parse_id(s, what)).collect()
}
