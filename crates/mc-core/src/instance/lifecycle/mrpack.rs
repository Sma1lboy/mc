use super::*;

// ===========================================================================
// 导入 / 导出
// ===========================================================================
//
// `modrinth.index.json` 的字段级模型已统一到 [`crate::modpack::formats::mrpack`];
// **导入的格式知识 + 解压 / 下载 / 装核心管线已上移到 [`crate::modpack::import`]**
// (可插拔架构),本模块的 [`import_mrpack`] 退化为薄包装。导出仍在本模块(把实例本地
// 游戏数据自包含打成 `.mrpack`)。

/// `modrinth.index.json` 在 `.mrpack` 内的固定路径(导出写索引用)。
pub(crate) const MRPACK_INDEX_ENTRY: &str = "modrinth.index.json";
/// 通用 overrides 目录前缀(导出把本地数据铺进它)。
pub(crate) const OVERRIDES_PREFIX: &str = "overrides/";

/// 导入一个 Modrinth `.mrpack` 到 `instance_id` 实例(**薄包装**)。
///
/// 自整合包导入做成可插拔架构(见 [`crate::modpack::import`])后,本函数不再各自实现
/// mrpack 解析 / 解压 / 下载,而是**委托给共享引擎** [`crate::modpack::import::ImportEngine`]:
/// 引擎用注册的 [`crate::modpack::import::modrinth::ModrinthImporter`] 探测 → `plan()` 解析
/// `modrinth.index.json` → 建实例 `versions/<instance_id>/` → **装原版 + loader**(`plan.loader`
/// 调现有 `loader::install_*`)→ 多源下载 `files[]`(sha512 强校验)→ 经 [`crate::fs::safe_join`]
/// 铺 `overrides/` 与 `client-overrides/`。
///
/// 与旧实现的差异(改进而非回退):**loader 现在一并安装**(旧版只装原版,把 loader 留给
/// 调用方),因为引擎对所有格式统一在第 7 步装核心。签名保持不变,既有调用方 / Tauri
/// 命令无需改动。
pub async fn import_mrpack(
    paths: &GamePaths,
    dl: &Downloader,
    mrpack_path: &Path,
    instance_id: &str,
) -> Result<()> {
    use crate::modpack::import::{ImportEngine, ImportOptions, ImportSource};
    use crate::modplatform::provider::ProviderRegistry;

    // mrpack 自带 URL,本身不需要 provider(resolve 为空操作);但用内建默认注册表
    // (Modrinth + 有 key 时的 CurseForge)以便同一引擎也能处理 curseforge / mcbbs 包。
    let engine = ImportEngine::with_defaults(dl.clone(), ProviderRegistry::with_defaults());
    let mut opts = ImportOptions::new(paths.root().to_path_buf());
    opts.instance_id = Some(instance_id.to_string());
    engine
        .import(ImportSource::LocalFile(mrpack_path.to_path_buf()), opts)
        .await
        .map(|_outcome| ())
}

// ===========================================================================
// 导出
// ===========================================================================

/// 实例里会被打进 `.mrpack` overrides 的游戏数据子目录。
const EXPORT_DIRS: &[&str] = &[
    "mods",
    "config",
    "resourcepacks",
    "shaderpacks",
    "datapacks",
    "scripts",
    "kubejs",
];

/// 把一个实例导出成最小可用的 Modrinth `.mrpack`(**自包含、无反查**)。
///
/// 生成内容:
/// - `modrinth.index.json`:`formatVersion=1`、`game="minecraft"`、`name=实例名`、
///   `dependencies={ "minecraft": mc_version }`、`files=[]`(本地 mod 无远程 url,
///   故全部以 overrides 形式内联,而不进 `files[]`)。
/// - `overrides/<dir>/...`:把实例下 [`EXPORT_DIRS`] 列出的子目录(mods/config/
///   resourcepacks…)递归打进 `overrides/` 下。
///
/// **与可插拔导出引擎的关系**:本函数是「不做平台反查」的快捷路径,复用
/// [`crate::modpack::export`] 的共享底座 —— 用 [`crate::modpack::export::modrinth::build_index`]
/// 生成索引(传入空的 [`ClassifiedSet`],故 `files` 为空)、用
/// [`crate::modpack::export::walk::walk_game_root`] 遍历各 `EXPORT_DIRS` 子树、用
/// [`crate::modpack::export::zip::write_zip`] 打包(排除集为空、注入索引、失败删半成品),
/// **不再**自带一份 zip 递归逻辑。需要「resolved 反查 → 小包」时改用
/// [`crate::modpack::export::ModpackExporter`] + [`crate::modpack::export::ModrinthExportTarget`]。
///
/// 写出到 `dest`。这是"自包含分发"的导出:接收方解压即得到完整文件,不依赖任何
/// 远程下载(代价是体积更大)。
pub fn export_mrpack(inst: &Instance, mc_version: &str, dest: &Path) -> Result<()> {
    use crate::modpack::export::walk::walk_game_root;
    use crate::modpack::export::zip::{write_zip, ZipPlan};
    use crate::modpack::export::{modrinth as export_modrinth, ClassifiedSet, ExportInput};

    let game_dir = inst.game_dir();

    // 实例名:优先 instance.json 的 name,缺省用版本 id。
    let name = inst
        .load_config()
        .ok()
        .and_then(|c| c.name)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| inst.version_id().to_string());

    // 遍历 EXPORT_DIRS 各子树,汇成相对 game_dir 的候选文件(套硬忽略 / path-safety)。
    // 自包含导出无反查:分类集留空(resolved 空 → 索引 files 空 + 无排除键)。
    let mut files = Vec::new();
    for sub in EXPORT_DIRS {
        let src = game_dir.join(sub);
        if !src.is_dir() {
            continue;
        }
        // walk 以 game_dir 为根算相对路径,但只遍历该子树:对整个 game_dir 走一遍再按前缀筛
        // 会重复扫描;这里直接以 game_dir 为根、把"非该子树"作为用户忽略不便,故按子树遍历后
        // 用 strip_prefix 拼回 game_dir 相对路径。
        let sub_files = walk_game_root(&src, &[])?;
        for mut f in sub_files {
            f.rel = format!("{sub}/{}", f.rel);
            files.push(f);
        }
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));

    // 用共享 build_index 生成 modrinth.index.json(空分类集 → files 为空)。
    let input = ExportInput {
        game_root: &game_dir,
        pack_name: name,
        pack_version: Some(inst.version_id().to_string()),
        summary: None,
        author: None,
        mc_version: mc_version.to_string(),
        loader: None,
        user_ignores: Vec::new(),
    };
    let empty = ClassifiedSet::default();
    let index = export_modrinth::build_index(&input, &empty);
    let index_bytes = serde_json::to_vec_pretty(&index)
        .map_err(|e| CoreError::Parse { what: MRPACK_INDEX_ENTRY.into(), source: e })?;

    let extra = vec![(MRPACK_INDEX_ENTRY.to_string(), index_bytes)];
    let exclude = std::collections::HashSet::new();
    let plan = ZipPlan {
        overrides_prefix: OVERRIDES_PREFIX,
        files: &files,
        exclude: &exclude,
        extra_files: &extra,
    };
    write_zip(dest, &plan)?;
    Ok(())
}
