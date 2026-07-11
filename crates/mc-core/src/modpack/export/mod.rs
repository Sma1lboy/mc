//! 可插拔整合包**导出核心**:一个共享导出引擎 + 每种目标格式一个 [`ExportTarget`]。
//!
//! 见 `docs/modules/modpack-export.md`。与[导入](super::import)对称:导入是
//! 「manifest → 实例」,导出是「实例 → 可分享归档」。两侧共用 [`ProviderRegistry`] 与
//! 哈希反查能力([`ResourceProvider::resolve_by_hashes`])。
//!
//! ## 核心难题:Resolvable vs Override
//!
//! 导出的全部技术含量在一句:**对每个文件,判定它能否表达成一个平台下载 URL**。
//! - **Resolvable**:哈希在平台(Modrinth / CurseForge)反查到对应版本 → 写进**索引**
//!   (URL/id),**不**进包(包又小又合规)。
//! - **Override**:反查不到 → 原样塞进 `overrides/`。
//!
//! 承重点:打 zip 时**排除** resolved 键(Prism 的 `setExcludeFiles`),resolved 文件只在索引里
//! 出现、不在 overrides 里重复(见 [`zip`])。
//!
//! ## 五阶段管线(引擎跑一次,目标只声明旋钮)
//!
//! 1. **collect**:[`walk::walk_game_root`] 遍历 game_root,套硬忽略 + 用户忽略 + path-safety。
//! 2. **hash**:对 `target.accepts()` 命中的候选,按 `target.hash_algo()` 算单一哈希。
//! 3. **resolve**:`provider.resolve_by_hashes(algo, hashes)` 批量反查;命中且 `allow_remote` 通过
//!    → Resolvable,否则回落 Override(安全默认)。
//! 4. **(可选)write_index**:目标基于分类集序列化索引字节(`modrinth.index.json` /
//!    `manifest.json` + `modlist.html` / 纯文本 modlist)。
//! 5. **package**:[`zip::write_zip`](排除 resolved 键 + 注入索引)或 [`zip::write_text_file`]。
//!
//! `provider() == None` 的目标(纯 modlist)自动跳过 2–3 阶段:所有候选都进 override 分类
//! (但 modlist 不打包 override,只输出文本)。

pub mod curseforge;
pub mod modlist;
pub mod modrinth;
pub mod walk;
pub mod zip;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mc_types::LoaderKind;

use crate::error::{CoreError, Result};
use crate::modplatform::provider::ProviderRegistry;
use crate::modplatform::{HashAlgo, ProviderId, ResolvedFile};

use walk::WalkedFile;

pub use curseforge::CurseForgeExportTarget;
pub use modlist::{ModListColumns, ModListExportTarget, ModListFormat};
pub use modrinth::ModrinthExportTarget;

// ===========================================================================
// 引擎输入
// ===========================================================================

/// 导出引擎的全部输入:实例游戏目录 + 元信息 + loader 图 + 用户忽略集。
///
/// 引擎只读不写实例;目标的 `write_index` 经此拿到 game_root 之外的全部上下文
/// (mc_version / loader / 包名作者等),自身保持 algo-无关。
pub struct ExportInput<'a> {
    /// 实例游戏目录(== version_dir),所有相对路径以它为根。
    pub game_root: &'a Path,
    /// 整合包名(写进索引)。
    pub pack_name: String,
    /// 整合包版本号(自由文本);无则索引里留空。
    pub pack_version: Option<String>,
    /// 简介(可选,Modrinth `summary`)。
    pub summary: Option<String>,
    /// 作者(CurseForge manifest / modlist 用;无则各格式给默认)。
    pub author: Option<String>,
    /// 目标 Minecraft 原版版本(如 `1.20.1`)。
    pub mc_version: String,
    /// loader 家族 + 版本;`None` 表示原版(无 loader 依赖)。来自实例的版本 profile / 溯源。
    pub loader: Option<(LoaderKind, String)>,
    /// 用户忽略的相对前缀集(来自 `<instance>/.packignore`);可空。
    pub user_ignores: Vec<String>,
}

impl<'a> ExportInput<'a> {
    /// 便捷构造:仅 game_root + 名 + mc_version,其余默认空。
    pub fn new(game_root: &'a Path, pack_name: impl Into<String>, mc_version: impl Into<String>) -> Self {
        ExportInput {
            game_root,
            pack_name: pack_name.into(),
            pack_version: None,
            summary: None,
            author: None,
            mc_version: mc_version.into(),
            loader: None,
            user_ignores: Vec::new(),
        }
    }
}

// ===========================================================================
// 分类结果
// ===========================================================================

/// 单个文件的分类。`Skipped` 用于门控未命中又被目标显式排除的极端情况(当前引擎不产出它,
/// 但保留枚举完整性以对齐设计文档,且让未来目标可声明跳过)。
#[derive(Debug, Clone)]
pub enum FileClass {
    /// 反查命中且允许远程引用 → 写进索引(携带解析结果)。
    Resolvable(Box<ResolvedFile>),
    /// 反查不到 / 不允许远程 → 进 `overrides/`(携带相对路径)。
    Override(PathBuf),
    /// 显式跳过(不进索引也不进 overrides)。
    Skipped,
}

/// 引擎产出的分类集:resolved(相对路径 + 解析结果)与 override(相对路径)两张表。
///
/// `resolved` 的 `PathBuf` 是相对 game_root 的路径(`/` 已归一为平台分隔符前的逻辑路径,
/// 内部以 `to_string_lossy` 取回 `/` 形式);目标的 `write_index` 用它生成索引 `path`,
/// 引擎用它生成 `exclude` 集喂给 zip。
#[derive(Debug, Clone, Default)]
pub struct ClassifiedSet {
    /// 可远程引用的文件:`(相对路径, 解析结果)`,按相对路径升序。
    pub resolved: Vec<(PathBuf, ResolvedFile)>,
    /// 必须随包带走的本地文件相对路径,按升序。
    pub overrides: Vec<PathBuf>,
}

impl ClassifiedSet {
    /// resolved 的相对路径键集合(`/` 分隔),即 zip 打包时要**排除**的 override 键。
    pub fn resolved_keys(&self) -> HashSet<String> {
        self.resolved
            .iter()
            .map(|(p, _)| p.to_string_lossy().replace('\\', "/"))
            .collect()
    }
}

// ===========================================================================
// 打包方式
// ===========================================================================

/// 目标的打包方式。`ZipWithOverrides` = mrpack / CF zip;`SingleTextFile` = modlist 文本。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packaging {
    /// zip + `overrides/`(排除 resolved 键)+ 注入索引文件。
    ZipWithOverrides,
    /// 单个文本文件(无 zip / 无 overrides;`write_index` 给唯一字节)。
    SingleTextFile,
}

/// 导出进度阶段(回调用;引擎在每阶段开始/推进时上报 `(done, total)`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPhase {
    /// 遍历 game_root 收集候选。
    Collect,
    /// 对门控命中的候选算哈希。
    Hash,
    /// 批量哈希反查(provider)。
    Resolve,
    /// (CurseForge)补 slug/name/authors。
    Backfill,
    /// 序列化索引 + 打包写盘。
    Package,
}

// ===========================================================================
// 可插拔目标
// ===========================================================================

/// 一种导出目标格式。引擎跑一次共享管线;每个格式只声明
/// 「门控 + 哈希算法 + 反查平台 + 远程引用策略 + 索引写法 + 打包方式」。对象安全。
pub trait ExportTarget: Send + Sync {
    /// 稳定 id(`"modrinth"` / `"curseforge"` / `"modlist"`)。
    fn id(&self) -> &'static str;

    /// 输出文件扩展名(不含点;`"mrpack"` / `"zip"` / `"html"` …)。
    fn output_extension(&self) -> &'static str;

    /// 反查用哪个平台;`None` = 跳过 resolve 阶段(纯 modlist)。
    fn provider(&self) -> Option<ProviderId>;

    /// 反查哈希算法(mrpack=Sha512;CurseForge=Murmur2);`provider()==None` 时应为 `None`。
    fn hash_algo(&self) -> Option<HashAlgo>;

    /// 文件门控:`relative`(相对 game_root)是否为本格式的候选反查文件(前缀 × 扩展名)。
    /// 返回 false 的文件不哈希、不反查,直接当 override(若打包)。
    fn accepts(&self, relative: &Path) -> bool;

    /// 已 resolved 的文件能否对本格式作**远程引用**?
    /// mrpack:仅当下载 host 在 mrpack 白名单内;CurseForge:仅当 `isAvailable`(有下载 URL)。
    /// 返回 false → 即便反查命中也强制塞进 `overrides/`。默认允许。
    fn allow_remote(&self, _r: &ResolvedFile) -> bool {
        true
    }

    /// 基于分类集 + 输入序列化索引,返回要注入归档的 `(归档内文件名, 字节)`。
    /// mrpack=1 个(`modrinth.index.json`);curseforge=2 个(`manifest.json` + `modlist.html`);
    /// modlist=1 个文本(其内容即最终输出文件)。
    fn write_index(&self, input: &ExportInput<'_>, set: &ClassifiedSet) -> Result<Vec<(String, Vec<u8>)>>;

    /// 打包方式。默认 zip + overrides。
    fn packaging(&self) -> Packaging {
        Packaging::ZipWithOverrides
    }
}

// ===========================================================================
// 引擎
// ===========================================================================

/// 共享导出引擎:持有 provider 注册表,对任意 [`ExportTarget`] 跑五阶段管线。
#[derive(Clone)]
pub struct ModpackExporter {
    providers: Arc<ProviderRegistry>,
}

impl ModpackExporter {
    /// 用一份 provider 注册表构造(导入 / 导出 / 浏览共用同一份)。
    pub fn new(providers: Arc<ProviderRegistry>) -> Self {
        Self { providers }
    }

    /// 用内建默认注册表构造(总有 Modrinth;有 `MC_CF_API_KEY` 时含 CurseForge)。
    pub fn with_defaults() -> Self {
        Self::new(Arc::new(ProviderRegistry::with_defaults()))
    }

    /// 把 `input` 描述的实例按 `target` 导出到 `dest`,返回写出的路径。
    ///
    /// `progress(phase, done, total)` 在每阶段推进时回调(可传 `|_,_,_| {}` 忽略)。
    /// 五阶段见模块文档;失败(读盘 / 网络 / 打包)会清理半成品并向上返回错误。
    pub async fn export(
        &self,
        target: &dyn ExportTarget,
        input: ExportInput<'_>,
        progress: &mut (dyn FnMut(ExportPhase, u64, u64) + Send),
    ) -> Result<PathBuf> {
        // 1) collect:遍历 + 忽略 + path-safety。
        progress(ExportPhase::Collect, 0, 1);
        let files = walk::walk_game_root(input.game_root, &input.user_ignores)?;
        progress(ExportPhase::Collect, 1, 1);

        // 2/3) 分类:门控命中的算哈希 → 反查 → allow_remote 过滤;其余进 override。
        let set = self
            .classify(target, &files, progress)
            .await?;

        // 4) 序列化索引(目标各自的 schema)。
        progress(ExportPhase::Package, 0, 1);
        let extra = target.write_index(&input, &set)?;

        // 5) 打包。
        let dest = self.resolve_dest(target, &input);
        let out = match target.packaging() {
            Packaging::ZipWithOverrides => {
                let exclude = set.resolved_keys();
                let plan = zip::ZipPlan {
                    overrides_prefix: "overrides/",
                    files: &files,
                    exclude: &exclude,
                    extra_files: &extra,
                };
                zip::write_zip(&dest, &plan)?
            }
            Packaging::SingleTextFile => {
                // modlist:write_index 必给恰好一个文本文件,其字节即最终输出。
                let (_, bytes) = extra
                    .first()
                    .ok_or_else(|| CoreError::other("modlist 导出未产出任何文本"))?;
                zip::write_text_file(&dest, bytes)?
            }
        };
        progress(ExportPhase::Package, 1, 1);
        Ok(out)
    }

    /// 由 game_root 的父目录 + 包名 + 目标扩展名推出默认输出路径。
    ///
    /// 例:`<root>/versions/<id>` 的实例、名 `My Pack`、mrpack → `<root>/versions/<id>/../My Pack.mrpack`
    /// 归一后落在实例目录旁。调用方也可在拿到 input 前自行指定;这里给一个合理默认。
    fn resolve_dest(&self, target: &dyn ExportTarget, input: &ExportInput<'_>) -> PathBuf {
        let safe = crate::fs::sanitize_filename(&input.pack_name, '_');
        let safe = if safe.trim().is_empty() { "modpack".to_string() } else { safe };
        let parent = input.game_root.parent().unwrap_or(input.game_root);
        parent.join(format!("{safe}.{}", target.output_extension()))
    }

    /// 第 2、3 阶段:门控 → 哈希 → 反查 → `allow_remote` → 分类。纯 IO/网络,无副作用落盘。
    async fn classify(
        &self,
        target: &dyn ExportTarget,
        files: &[WalkedFile],
        progress: &mut (dyn FnMut(ExportPhase, u64, u64) + Send),
    ) -> Result<ClassifiedSet> {
        // 门控:命中者进 resolve 候选,其余直接 override。
        let mut gate_hits: Vec<&WalkedFile> = Vec::new();
        let mut overrides: Vec<PathBuf> = Vec::new();
        for f in files {
            if target.accepts(Path::new(&f.rel)) {
                gate_hits.push(f);
            } else {
                overrides.push(PathBuf::from(&f.rel));
            }
        }

        // provider == None(纯 modlist):无 resolve,门控命中也全进 override(modlist 不打包,
        // 但 classify 仍要把它们归入 override 以保持语义统一;modlist write_index 只读 resolved
        // 为空、从 override 走不通——故 modlist 目标 accepts 恒 false,这里 gate_hits 必空)。
        let (algo, provider) = match (target.hash_algo(), target.provider()) {
            (Some(algo), Some(pid)) => {
                let provider = self.providers.get(pid).ok_or_else(|| {
                    CoreError::other(format!(
                        "导出目标 {} 需要 {:?} provider,但未注册(CurseForge 需配 MC_CF_API_KEY)",
                        target.id(),
                        pid
                    ))
                })?;
                // 断言 provider 支持该算法(引擎保持 algo-无关,只校验能力)。
                if !provider.caps().hash_algos.contains(&algo) {
                    return Err(CoreError::other(format!(
                        "provider {:?} 不支持哈希算法 {:?}",
                        pid, algo
                    )));
                }
                (algo, Some(provider))
            }
            _ => {
                // 无 provider:门控命中的也降级为 override(纯文本目标不依赖反查)。
                for f in gate_hits {
                    overrides.push(PathBuf::from(&f.rel));
                }
                overrides.sort();
                return Ok(ClassifiedSet { resolved: Vec::new(), overrides });
            }
        };
        let provider = provider.expect("provider present in resolve branch");

        // 2) hash:对门控命中者算单一哈希(与下标对齐)。
        let total = gate_hits.len() as u64;
        progress(ExportPhase::Hash, 0, total);
        let mut hashes: Vec<String> = Vec::with_capacity(gate_hits.len());
        for (i, f) in gate_hits.iter().enumerate() {
            hashes.push(f.hash(algo)?);
            progress(ExportPhase::Hash, (i + 1) as u64, total);
        }

        // 3) resolve:批量反查。空集免联网。
        let mut resolved: Vec<(PathBuf, ResolvedFile)> = Vec::new();
        if !hashes.is_empty() {
            progress(ExportPhase::Resolve, 0, total);
            let matches = provider.resolve_by_hashes(algo, &hashes).await?;
            // 防御:长度对齐(provider 契约保证,这里兜底)。
            for (i, f) in gate_hits.iter().enumerate() {
                let resolved_file = matches.get(i).and_then(|m| m.clone());
                match resolved_file {
                    // 命中且允许远程 → Resolvable。
                    Some(r) if target.allow_remote(&r) => {
                        resolved.push((PathBuf::from(&f.rel), r));
                    }
                    // 命中但不允许远程(host 不在白名单 / 不可用)→ 回落 override(安全默认)。
                    Some(_) => overrides.push(PathBuf::from(&f.rel)),
                    // 未命中 → override。
                    None => overrides.push(PathBuf::from(&f.rel)),
                }
            }
            progress(ExportPhase::Resolve, total, total);
        }

        resolved.sort_by(|a, b| a.0.cmp(&b.0));
        overrides.sort();
        Ok(ClassifiedSet { resolved, overrides })
    }
}

#[cfg(test)]
mod tests;
