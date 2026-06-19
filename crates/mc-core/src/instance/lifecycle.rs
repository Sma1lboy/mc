//! 实例生命周期(复制 / 删除)与 Modrinth 整合包(`.mrpack`)的导入 / 导出。
//!
//! 在"版本即实例"模型下,一个实例就是一个 `versions/<id>/` 目录:版本 json
//! (`versions/<id>/<id>.json`)、可选的客户端 jar(`versions/<id>/<id>.jar`)、
//! `instance.json`,以及运行时游戏数据(mods/saves/config/resourcepacks…)全部
//! 平铺在该目录下(game_dir == version_dir)。本模块围绕这一布局提供四个操作:
//!
//! - [`copy_instance`]:整目录复制为新 id,并把版本 json/jar 的内部 id 一并改名,
//!   使复制出的实例自洽可启动。
//! - [`delete_instance`]:把整个实例目录移入回收站(失败回退到不可逆删除)。
//! - [`import_mrpack`]:解析 Modrinth `.mrpack`(本质是 zip),安装其声明的原版
//!   Minecraft / loader、覆盖 `overrides/`、下载 `files[]` 到实例目录。
//! - [`export_mrpack`]:把实例的本地游戏数据(mods/config/resourcepacks…)打成一个
//!   最小可用的 `.mrpack`,所有本地文件都放进 `overrides/` 下(无远程 url 时的兜底)。
//!
//! 设计要点:
//! - **zip-slip 防护**:解压 `files[]` 与 `overrides/` 里的相对路径前都用
//!   [`crate::fs::safe_join`] 收口到实例目录内,拒绝 `../` 越权。
//! - **env 容错**:`files[].env.client == "unsupported"` 的文件(纯服务端文件)在
//!   客户端导入时跳过,不下载。
//! - **保真落盘**:版本 json 用 `serde_json::Value` 只改 `id` 字段后写出,其余字段
//!   原样保留(避免重新序列化丢失未建模字段)。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use serde::Deserialize;

use crate::download::{DownloadItem, Downloader};
use crate::error::{CoreError, IoResultExt, Result};
use crate::instance::Instance;
use crate::paths::{ensure_dir, GamePaths};

// ===========================================================================
// 复制 / 删除
// ===========================================================================

/// 把实例 `src_id` 整目录复制为新实例 `new_id`。
///
/// 步骤:
/// 1. 校验:`src_id` 必须存在(有 `versions/<src_id>/<src_id>.json`);`new_id`
///    目录必须不存在(否则返回 [`CoreError::Other`],拒绝覆盖)。
/// 2. 用 [`crate::fs::copy_dir`] 递归复制 `versions/<src_id>/` → `versions/<new_id>/`。
/// 3. 修正复制出的目录里跟 id 绑定的文件名与内容:
///    - 版本 json:读出后把内部 `"id"` 字段改成 `new_id`,写到
///      `versions/<new_id>/<new_id>.json`,并删除沿用旧名的 `<src_id>.json`。
///    - 客户端 jar:若存在 `<src_id>.jar`,改名为 `<new_id>.jar`。
///
/// 复制 instance.json 与 mods/saves/config/resourcepacks 等游戏数据都由第 2 步的整
/// 目录复制覆盖,无需逐类处理。
pub fn copy_instance(paths: &GamePaths, src_id: &str, new_id: &str) -> Result<()> {
    let src_dir = paths.version_dir(src_id);
    let dst_dir = paths.version_dir(new_id);

    // 源必须是一个真正的实例(存在版本 json)。
    let src_json = paths.version_json(src_id);
    if !src_json.is_file() {
        return Err(CoreError::other(format!(
            "源实例 {src_id} 不存在(缺少 {})",
            src_json.display()
        )));
    }
    // 目标不能已存在,避免静默覆盖另一个实例。
    if dst_dir.exists() {
        return Err(CoreError::other(format!("目标实例 {new_id} 已存在,无法复制")));
    }

    // 1) 整目录递归复制(含 mods/saves/config/resourcepacks/instance.json 等)。
    crate::fs::copy_dir(&src_dir, &dst_dir)?;

    // 2) 改写版本 json:把内部 id 改成 new_id 并落到新名文件,删掉沿用旧名的那份。
    //    复制后此刻磁盘上是 versions/<new_id>/<src_id>.json(内容里 id 仍是 src_id)。
    let copied_old_json = dst_dir.join(format!("{src_id}.json"));
    let new_json = paths.version_json(new_id);
    let raw = std::fs::read_to_string(&copied_old_json).with_path(&copied_old_json)?;
    let rewritten = rewrite_version_id(&raw, new_id)?;
    crate::fs::write_atomic(&new_json, rewritten.as_bytes())?;
    // 删除旧名 json(若新旧同名则上一步已覆盖,无需也不会误删)。
    if copied_old_json != new_json {
        std::fs::remove_file(&copied_old_json).with_path(&copied_old_json)?;
    }

    // 3) 客户端 jar 随 id 改名(jar 内容与 id 无关,仅文件名需匹配 <new_id>.jar)。
    let copied_old_jar = dst_dir.join(format!("{src_id}.jar"));
    let new_jar = paths.version_jar(new_id);
    if copied_old_jar.is_file() && copied_old_jar != new_jar {
        std::fs::rename(&copied_old_jar, &new_jar).with_path(&copied_old_jar)?;
    }

    Ok(())
}

/// 把版本 json 文本里的顶层 `"id"` 字段改成 `new_id`,返回改写后的 json 文本。
///
/// 用 [`serde_json::Value`] 做最小改动:只 insert/替换 `id` 这一个键,其余字段
/// (inheritsFrom / libraries / arguments / downloads…)原样保留并重新序列化。
fn rewrite_version_id(raw: &str, new_id: &str) -> Result<String> {
    let mut value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| CoreError::Parse { what: "version json".into(), source: e })?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| CoreError::other("version json 顶层不是对象,无法改写 id"))?;
    obj.insert("id".to_string(), serde_json::Value::String(new_id.to_string()));
    serde_json::to_string_pretty(&value)
        .map_err(|e| CoreError::Parse { what: "version json".into(), source: e })
}

/// 删除整个实例目录(`versions/<id>/`)。
///
/// 优先用 [`trash::delete`] 移入系统回收站(可恢复,符合"删除实例"这类高代价、
/// 用户可逆操作的预期);在无 GUI / 不支持回收站的环境回退到
/// [`std::fs::remove_dir_all`] 永久删除。目录不存在视为已删除(幂等),不报错。
pub fn delete_instance(paths: &GamePaths, id: &str) -> Result<()> {
    let dir = paths.version_dir(id);
    if !dir.exists() {
        return Ok(());
    }
    if trash::delete(&dir).is_ok() {
        return Ok(());
    }
    std::fs::remove_dir_all(&dir).with_path(&dir)
}

// ===========================================================================
// Modrinth .mrpack 索引模型
// ===========================================================================

/// `modrinth.index.json` 顶层结构。字段名严格对齐 Modrinth modpack 格式
/// (<https://docs.modrinth.com/modpacks/format/>)。缺省字段一律 `#[serde(default)]`,
/// 不让单个字段缺失把整次导入打挂。
#[derive(Debug, Clone, Deserialize)]
struct MrpackIndex {
    /// 格式版本,当前应为 1。仅记录,不强校验(向后兼容未来版本)。
    #[serde(default, rename = "formatVersion")]
    format_version: u32,
    /// 整合包名。导入时写入实例 `instance.json` 的 name。
    #[serde(default)]
    name: String,
    /// 依赖:必含 `minecraft`,可含 `fabric-loader`/`quilt-loader`/`forge`/`neoforge`。
    #[serde(default)]
    dependencies: std::collections::BTreeMap<String, String>,
    /// 受管理的远程文件列表(mods / 配置等带下载地址的文件)。
    #[serde(default)]
    files: Vec<MrpackFile>,
}

/// `files[]` 中的单个受管理文件。
#[derive(Debug, Clone, Deserialize)]
struct MrpackFile {
    /// 相对实例根目录的落盘路径,如 `mods/sodium.jar`。
    #[serde(default)]
    path: String,
    /// 下载地址列表,取第 0 个为主源。
    #[serde(default)]
    downloads: Vec<String>,
    /// 校验和;我们用 `sha1`(若有)做下载后校验。
    #[serde(default)]
    hashes: MrpackHashes,
    /// 文件大小(字节),仅用于进度展示。
    #[serde(default, rename = "fileSize")]
    file_size: Option<u64>,
    /// 环境适用性:`{ "client": "required|optional|unsupported", "server": ... }`。
    /// 客户端导入时跳过 `client == "unsupported"` 的文件。
    #[serde(default)]
    env: MrpackEnv,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct MrpackHashes {
    #[serde(default)]
    sha1: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct MrpackEnv {
    #[serde(default)]
    client: Option<String>,
}

impl MrpackFile {
    /// 该文件在客户端是否受支持(用于跳过纯服务端文件)。
    fn client_supported(&self) -> bool {
        // 仅当显式标注 client == "unsupported" 时跳过;缺省 / 其它取值都视为需要。
        self.env.client.as_deref() != Some("unsupported")
    }
}

// ===========================================================================
// 导入
// ===========================================================================

/// `modrinth.index.json` 在 `.mrpack` 内的固定路径。
const MRPACK_INDEX_ENTRY: &str = "modrinth.index.json";
/// 通用 overrides 目录前缀(客户端 + 服务端共用)。
const OVERRIDES_PREFIX: &str = "overrides/";
/// 客户端专属 overrides 目录前缀。
const CLIENT_OVERRIDES_PREFIX: &str = "client-overrides/";

/// 导入一个 Modrinth `.mrpack` 到 `instance_id` 实例。
///
/// 流程:
/// 1. 读 `modrinth.index.json`,取 `dependencies.minecraft` 作为原版版本,从
///    [`crate::meta::fetch_manifest`] 找到对应清单条目并 [`crate::launch::install_version`]
///    安装原版(版本目录即 `versions/<minecraft>/`)。**整合包实例本身**建在
///    `versions/<instance_id>/`(= game_dir),其 `instance.json` 写入整合包名。
/// 2. 解压 zip 里的 `overrides/` 与 `client-overrides/`,经 [`crate::fs::safe_join`]
///    收口后覆盖到实例 game_dir。
/// 3. 下载 `files[]`(`downloads[0]` 为 url、`hashes.sha1` 校验)到
///    `versions/<instance_id>/<file.path>`;跳过 `env.client == "unsupported"` 的文件。
///
/// 若包声明 loader,本函数会先安装 loader,再写入一个继承该 loader profile 的
/// 实例专属版本 json,从而让 `versions/<instance_id>` 既是启动 profile,也是 game_dir。
pub async fn import_mrpack(
    paths: &GamePaths,
    dl: &Downloader,
    mrpack_path: &Path,
    instance_id: &str,
) -> Result<()> {
    // ---- 1) 打开 zip 并读取索引 ----
    let file = std::fs::File::open(mrpack_path).with_path(mrpack_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Zip(e.to_string()))?;

    let index: MrpackIndex = {
        let mut entry = archive
            .by_name(MRPACK_INDEX_ENTRY)
            .map_err(|_| CoreError::other(format!(".mrpack 缺少 {MRPACK_INDEX_ENTRY}")))?;
        let mut raw = String::new();
        entry
            .read_to_string(&mut raw)
            .map_err(|e| CoreError::Zip(format!("读取 {MRPACK_INDEX_ENTRY} 失败: {e}")))?;
        serde_json::from_str(&raw)
            .map_err(|e| CoreError::Parse { what: MRPACK_INDEX_ENTRY.into(), source: e })?
    };

    // formatVersion 当前规范为 1;遇到未知版本只记日志、不中断(向后兼容,字段语义
    // 至今未发生破坏性变更)。
    if index.format_version != 1 {
        tracing::warn!(
            format_version = index.format_version,
            "未知的 mrpack formatVersion,按 v1 语义继续导入"
        );
    }

    // ---- 2) 安装索引声明的原版 Minecraft / loader ----
    let mc_version = index
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or_else(|| CoreError::other(".mrpack dependencies 缺少 minecraft 版本"))?;

    // 取 Mojang 清单条目:原版安装与 loader 安装都需要它。
    let manifest = crate::meta::fetch_manifest(dl).await?;
    let vanilla_entry = manifest
        .iter()
        .find(|m| m.id == mc_version)
        .ok_or_else(|| CoreError::other(format!("版本清单中找不到 Minecraft {mc_version}")))?
        .clone();

    // 仅当原版尚未安装时才安装。
    if !paths.version_json(&mc_version).is_file() {
        crate::launch::install_version(dl, paths, &vanilla_entry, None).await?;
    }

    let base_profile_id =
        install_declared_loader(&index, dl, paths, &mc_version, &vanilla_entry).await?;

    // ---- 3) 准备整合包实例目录(game_dir == versions/<instance_id>/) ----
    let inst = Instance::new(instance_id, paths.root().to_path_buf());
    let game_dir = inst.game_dir();
    ensure_dir(&game_dir)?;

    // 写一个实例专属 profile,让版本即实例模型能用自定义 instance_id 承载整合包。
    // 这个 profile 继承原版或 loader profile,实际 game_dir 仍指向 versions/<instance_id>/。
    write_instance_profile(paths, instance_id, &base_profile_id)?;

    // 写实例配置:把整合包名记到 instance.json(便于实例列表展示)。
    if !index.name.is_empty() {
        let mut config = inst.load_config().unwrap_or_default();
        config.name = Some(index.name.clone());
        inst.save_config(&config)?;
    }

    // ---- 4) 解压 overrides/ 与 client-overrides/ 到 game_dir ----
    extract_overrides(&mut archive, &game_dir)?;

    // ---- 5) 下载受管理文件 files[] ----
    let mut items: Vec<DownloadItem> = Vec::new();
    for f in &index.files {
        // 跳过纯服务端文件(客户端不受支持)。
        if !f.client_supported() {
            continue;
        }
        // 没有任何下载源的条目无法处理,跳过(overrides 已覆盖这类本地文件)。
        let Some(url) = f.downloads.first() else { continue };
        // zip-slip 防护:path 必须收口在实例目录内。
        let Some(dest) = crate::fs::safe_join(&game_dir, &f.path) else {
            return Err(CoreError::other(format!("非法的整合包文件路径(越权): {}", f.path)));
        };
        items.push(DownloadItem {
            url: url.clone(),
            path: dest,
            sha1: f.hashes.sha1.clone(),
            size: f.file_size,
        });
    }
    if !items.is_empty() {
        dl.download_all(items, None).await?;
    }

    Ok(())
}

async fn install_declared_loader(
    index: &MrpackIndex,
    dl: &Downloader,
    paths: &GamePaths,
    mc_version: &str,
    vanilla_entry: &mc_types::ManifestVersion,
) -> Result<String> {
    if let Some(loader_version) = index.dependencies.get("fabric-loader") {
        return crate::loader::install_fabric_version(
            dl,
            paths,
            mc_version,
            vanilla_entry,
            Some(loader_version.as_str()),
            None,
        )
        .await;
    }

    if let Some(loader_version) = index.dependencies.get("quilt-loader") {
        return crate::loader::install_quilt_version(
            dl,
            paths,
            mc_version,
            vanilla_entry,
            Some(loader_version.as_str()),
            None,
        )
        .await;
    }

    if let Some(neo_version) = index.dependencies.get("neoforge") {
        return crate::loader::install_neoforge(dl, paths, neo_version, vanilla_entry, None, None)
            .await;
    }

    if let Some(forge_build) = index.dependencies.get("forge") {
        return crate::loader::install_forge(
            dl,
            paths,
            mc_version,
            forge_build,
            vanilla_entry,
            None,
            None,
        )
        .await;
    }

    Ok(mc_version.to_string())
}

fn write_instance_profile(paths: &GamePaths, instance_id: &str, base_profile_id: &str) -> Result<()> {
    let profile = serde_json::json!({
        "id": instance_id,
        "inheritsFrom": base_profile_id,
        "type": "release",
    });
    let raw = serde_json::to_vec_pretty(&profile)
        .map_err(|e| CoreError::Parse { what: "instance profile json".into(), source: e })?;
    ensure_dir(&paths.version_dir(instance_id))?;
    crate::fs::write_atomic(&paths.version_json(instance_id), &raw)
}

/// 把 zip 内 `overrides/` 与 `client-overrides/` 下的所有条目解压到 `game_dir`。
///
/// 分两遍写入:先通用 `overrides/`,再 `client-overrides/`。第二遍覆盖第一遍的同名
/// 文件,从而在两者冲突时由客户端专属版本胜出(符合 Modrinth 约定)。每个条目都经
/// [`crate::fs::safe_join`] 收口,拒绝越权路径(zip-slip)。
fn extract_overrides(
    archive: &mut zip::ZipArchive<std::fs::File>,
    game_dir: &Path,
) -> Result<()> {
    write_override_pass(archive, game_dir, OVERRIDES_PREFIX)?;
    write_override_pass(archive, game_dir, CLIENT_OVERRIDES_PREFIX)?;
    Ok(())
}

/// 解压 zip 内带某个前缀的所有文件条目到 `game_dir`(单遍)。
fn write_override_pass(
    archive: &mut zip::ZipArchive<std::fs::File>,
    game_dir: &Path,
    prefix: &str,
) -> Result<()> {
    // 先扫描出命中前缀的 (index, 相对路径),再逐个写出(避免可变借用冲突)。
    let mut targets: Vec<(usize, String)> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| CoreError::Zip(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if let Some(rel) = name.strip_prefix(prefix) {
            if !rel.is_empty() {
                targets.push((i, rel.to_string()));
            }
        }
    }

    for (i, rel) in targets {
        // zip-slip 防护。
        let Some(dest) = crate::fs::safe_join(game_dir, &rel) else {
            return Err(CoreError::other(format!("非法的 overrides 路径(越权): {rel}")));
        };
        if let Some(parent) = dest.parent() {
            ensure_dir(parent)?;
        }
        let mut entry = archive.by_index(i).map_err(|e| CoreError::Zip(e.to_string()))?;
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut buf)
            .map_err(|e| CoreError::Zip(format!("读取 overrides 条目失败: {e}")))?;
        // 覆盖写(client-overrides 第二遍会覆盖同名 overrides)。
        crate::fs::write_atomic(&dest, &buf)?;
    }
    Ok(())
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

/// 把一个实例导出成最小可用的 Modrinth `.mrpack`。
///
/// 生成内容:
/// - `modrinth.index.json`:`formatVersion=1`、`game="minecraft"`、`name=实例名`、
///   `dependencies` 包含 Minecraft 与可识别的 loader、`files=[]`(本地 mod 无远程
///   url,故全部以 overrides 形式内联,而不进 `files[]`)。
/// - `overrides/<dir>/...`:把实例下 [`EXPORT_DIRS`] 列出的子目录(mods/config/
///   resourcepacks…)递归打进 `overrides/` 下。
///
/// 写出到 `dest`。这是"自包含分发"的导出:接收方解压即得到完整文件,不依赖任何
/// 远程下载(代价是体积更大)。
pub fn export_mrpack(inst: &Instance, mc_version: &str, dest: &Path) -> Result<()> {
    let game_dir = inst.game_dir();

    // 实例名:优先 instance.json 的 name,缺省用版本 id。
    let name = inst
        .load_config()
        .ok()
        .and_then(|c| c.name)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| inst.version_id().to_string());
    let dependencies = mrpack_dependencies(inst, mc_version);

    // 构造 modrinth.index.json(files 为空数组:所有本地文件走 overrides)。
    let index = serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": inst.version_id(),
        "name": name,
        "dependencies": dependencies,
        "files": [],
    });
    let index_bytes = serde_json::to_vec_pretty(&index)
        .map_err(|e| CoreError::Parse { what: "modrinth.index.json".into(), source: e })?;

    // 创建目标 .mrpack(zip)。
    if let Some(parent) = dest.parent() {
        ensure_dir(parent)?;
    }
    let out = std::fs::File::create(dest).with_path(dest)?;
    let mut writer = zip::ZipWriter::new(out);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // 1) 写索引。
    writer
        .start_file(MRPACK_INDEX_ENTRY, options)
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    writer
        .write_all(&index_bytes)
        .map_err(|e| CoreError::Zip(e.to_string()))?;

    // 2) 写 overrides/<dir>/...(仅打包实际存在的目录)。
    for sub in EXPORT_DIRS {
        let src = game_dir.join(sub);
        if !src.is_dir() {
            continue;
        }
        let zip_root = format!("{OVERRIDES_PREFIX}{sub}");
        zip_dir_recursive(&mut writer, &src, &src, &zip_root, options)?;
    }

    writer.finish().map_err(|e| CoreError::Zip(e.to_string()))?;
    Ok(())
}

fn mrpack_dependencies(inst: &Instance, mc_version: &str) -> BTreeMap<String, String> {
    let mut dependencies = BTreeMap::new();
    dependencies.insert("minecraft".to_string(), mc_version.to_string());

    if let Some((key, version)) = detect_mrpack_loader_dependency(inst, mc_version) {
        dependencies.insert(key.to_string(), version);
    }

    dependencies
}

fn detect_mrpack_loader_dependency(
    inst: &Instance,
    mc_version: &str,
) -> Option<(&'static str, String)> {
    let paths = inst.paths();
    let profile = match crate::launch::resolve_disk_profile(&paths, inst.version_id()) {
        Ok(profile) => profile,
        Err(_) => return detect_loader_dependency_from_id(inst.version_id(), mc_version),
    };

    let mut fabric = None;
    let mut quilt = None;
    let mut neoforge = None;
    let mut forge = None;

    for lib in &profile.libraries {
        let Some(spec) = lib.spec() else { continue };
        match (spec.group.as_str(), spec.artifact.as_str()) {
            ("net.fabricmc", "fabric-loader") => {
                fabric.get_or_insert(spec.version.clone());
            }
            ("org.quiltmc", "quilt-loader") => {
                quilt.get_or_insert(spec.version.clone());
            }
            ("net.neoforged", "neoforge") => {
                neoforge.get_or_insert(spec.version.clone());
            }
            ("net.minecraftforge", "forge") => {
                let prefix = format!("{mc_version}-");
                let build = spec.version.strip_prefix(&prefix).unwrap_or(&spec.version);
                forge.get_or_insert(build.to_string());
            }
            _ => {}
        }
    }

    fabric
        .map(|v| ("fabric-loader", v))
        .or_else(|| quilt.map(|v| ("quilt-loader", v)))
        .or_else(|| neoforge.map(|v| ("neoforge", v)))
        .or_else(|| forge.map(|v| ("forge", v)))
        .or_else(|| detect_loader_dependency_from_id(&profile.id, mc_version))
}

fn detect_loader_dependency_from_id(id: &str, mc_version: &str) -> Option<(&'static str, String)> {
    extract_between_loader_marker(id, "fabric-loader-", mc_version)
        .map(|v| ("fabric-loader", v))
        .or_else(|| {
            extract_between_loader_marker(id, "quilt-loader-", mc_version)
                .map(|v| ("quilt-loader", v))
        })
        .or_else(|| extract_after_loader_marker(id, "neoforge-").map(|v| ("neoforge", v)))
        .or_else(|| extract_after_loader_marker(id, "-forge-").map(|v| ("forge", v)))
}

fn extract_between_loader_marker(id: &str, marker: &str, mc_version: &str) -> Option<String> {
    let lower = id.to_ascii_lowercase();
    let start = lower.find(marker)? + marker.len();
    let rest = &id[start..];
    let suffix = format!("-{mc_version}");
    Some(rest.strip_suffix(&suffix).unwrap_or(rest).to_string())
        .filter(|v| !v.is_empty())
}

fn extract_after_loader_marker(id: &str, marker: &str) -> Option<String> {
    let lower = id.to_ascii_lowercase();
    let start = lower.find(marker)? + marker.len();
    Some(id[start..].to_string()).filter(|v| !v.is_empty())
}

/// 把 `current` 目录递归写进 zip,内部路径前缀为 `zip_root`(相对 `base` 拼接)。
///
/// 例如 `base = .../mods`、`zip_root = "overrides/mods"`,则 `.../mods/a/b.jar`
/// 会写成 zip 内的 `overrides/mods/a/b.jar`。zip 路径分隔符统一为 `/`。
fn zip_dir_recursive<W: std::io::Write + std::io::Seek>(
    writer: &mut zip::ZipWriter<W>,
    base: &Path,
    current: &Path,
    zip_root: &str,
    options: zip::write::SimpleFileOptions,
) -> Result<()> {
    for entry in std::fs::read_dir(current).with_path(current)? {
        let entry = entry.with_path(current)?;
        let path = entry.path();
        // 相对 base 的子路径,拼到 zip_root 之后。
        let rel = path.strip_prefix(base).unwrap_or(&path);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let name = if rel_str.is_empty() {
            zip_root.to_string()
        } else {
            format!("{zip_root}/{rel_str}")
        };

        let ft = entry.file_type().with_path(&path)?;
        if ft.is_dir() {
            writer
                .add_directory(format!("{name}/"), options)
                .map_err(|e| CoreError::Zip(e.to_string()))?;
            zip_dir_recursive(writer, base, &path, zip_root, options)?;
        } else if ft.is_file() {
            writer
                .start_file(name, options)
                .map_err(|e| CoreError::Zip(e.to_string()))?;
            let data = std::fs::read(&path).with_path(&path)?;
            writer.write_all(&data).map_err(|e| CoreError::io(&path, e))?;
        }
        // 符号链接等其它类型不打包。
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// 临时 game root,Drop 时自动清理。
    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("mc-core-lifecycle-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn paths(&self) -> GamePaths {
            GamePaths::new(self.path.clone())
        }

        fn add_version_json(&self, id: &str, raw: &str) {
            let paths = self.paths();
            fs::create_dir_all(paths.version_dir(id)).unwrap();
            fs::write(paths.version_json(id), raw).unwrap();
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    // ---- copy_instance ----

    #[test]
    fn copy_instance_rewrites_id_and_renames_files() {
        let root = TempRoot::new("copy");
        let paths = root.paths();

        // 造一个源实例:版本 json + jar + 一个 mod + instance.json。
        let src_dir = paths.version_dir("1.20.1");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            paths.version_json("1.20.1"),
            r#"{"id":"1.20.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )
        .unwrap();
        fs::write(paths.version_jar("1.20.1"), b"FAKEJAR").unwrap();
        fs::create_dir_all(src_dir.join("mods")).unwrap();
        fs::write(src_dir.join("mods/sodium.jar"), b"MODBYTES").unwrap();
        fs::write(src_dir.join("instance.json"), r#"{"name":"Source","memory_mb":4096}"#).unwrap();

        copy_instance(&paths, "1.20.1", "my-copy").unwrap();

        let dst_dir = paths.version_dir("my-copy");
        // 新名 json 存在、旧名 json 不存在。
        let new_json = paths.version_json("my-copy");
        assert!(new_json.is_file(), "应生成 my-copy.json");
        assert!(!dst_dir.join("1.20.1.json").exists(), "旧名 json 应被删除");

        // json 内部 id 已改写,其余字段保留。
        let parsed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&new_json).unwrap()).unwrap();
        assert_eq!(parsed["id"], "my-copy");
        assert_eq!(parsed["mainClass"], "net.minecraft.client.main.Main");
        assert_eq!(parsed["type"], "release");

        // jar 随 id 改名。
        assert!(paths.version_jar("my-copy").is_file(), "jar 应改名为 my-copy.jar");
        assert!(!dst_dir.join("1.20.1.jar").exists());

        // 游戏数据 + instance.json 被复制。
        assert_eq!(fs::read(dst_dir.join("mods/sodium.jar")).unwrap(), b"MODBYTES");
        assert!(dst_dir.join("instance.json").is_file());

        // 源实例保持原样(复制而非移动)。
        assert!(paths.version_json("1.20.1").is_file());
        assert!(paths.version_jar("1.20.1").is_file());
    }

    #[test]
    fn copy_instance_rejects_existing_target() {
        let root = TempRoot::new("copy-exists");
        let paths = root.paths();

        fs::create_dir_all(paths.version_dir("a")).unwrap();
        fs::write(paths.version_json("a"), r#"{"id":"a"}"#).unwrap();
        // 目标已存在。
        fs::create_dir_all(paths.version_dir("b")).unwrap();

        let err = copy_instance(&paths, "a", "b").unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "目标已存在应返回 Other 错误");
    }

    #[test]
    fn copy_instance_rejects_missing_source() {
        let root = TempRoot::new("copy-missing");
        let paths = root.paths();
        let err = copy_instance(&paths, "nope", "dst").unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "源不存在应返回 Other 错误");
    }

    #[test]
    fn copy_instance_without_jar_succeeds() {
        // 没有客户端 jar(如纯继承的 loader profile)时,复制仍应成功。
        let root = TempRoot::new("copy-nojar");
        let paths = root.paths();
        fs::create_dir_all(paths.version_dir("src")).unwrap();
        fs::write(paths.version_json("src"), r#"{"id":"src","inheritsFrom":"1.20.1"}"#).unwrap();

        copy_instance(&paths, "src", "dst").unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(paths.version_json("dst")).unwrap()).unwrap();
        assert_eq!(parsed["id"], "dst");
        assert_eq!(parsed["inheritsFrom"], "1.20.1");
        assert!(!paths.version_jar("dst").exists(), "源无 jar 时目标也不应有 jar");
    }

    // ---- rewrite_version_id ----

    #[test]
    fn rewrite_version_id_preserves_other_fields() {
        let raw = r#"{
            "id": "old-id",
            "inheritsFrom": "1.20.1",
            "type": "release",
            "libraries": [{"name": "a:b:1"}],
            "arguments": {"game": ["--foo"]}
        }"#;
        let out = rewrite_version_id(raw, "new-id").unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["id"], "new-id");
        assert_eq!(v["inheritsFrom"], "1.20.1");
        assert_eq!(v["type"], "release");
        assert_eq!(v["libraries"][0]["name"], "a:b:1");
        assert_eq!(v["arguments"]["game"][0], "--foo");
    }

    #[test]
    fn rewrite_version_id_rejects_non_object() {
        let err = rewrite_version_id("[1,2,3]", "x").unwrap_err();
        assert!(matches!(err, CoreError::Other(_)));
    }

    // ---- delete_instance ----

    #[test]
    fn delete_instance_removes_dir_and_is_idempotent() {
        let root = TempRoot::new("delete");
        let paths = root.paths();
        let dir = paths.version_dir("doomed");
        fs::create_dir_all(&dir).unwrap();
        fs::write(paths.version_json("doomed"), r#"{"id":"doomed"}"#).unwrap();

        delete_instance(&paths, "doomed").unwrap();
        assert!(!dir.exists(), "删除后目录应不在原位(回收站或硬删均可)");

        // 重复删除不存在的实例应幂等成功。
        delete_instance(&paths, "doomed").unwrap();
    }

    // ---- mrpack 索引解析 ----

    #[test]
    fn parse_mrpack_index_inline() {
        let sample = r#"{
            "formatVersion": 1,
            "game": "minecraft",
            "name": "My Modpack",
            "versionId": "1.0.0",
            "dependencies": {
                "minecraft": "1.20.1",
                "fabric-loader": "0.15.7"
            },
            "files": [
                {
                    "path": "mods/sodium.jar",
                    "downloads": ["https://cdn.modrinth.com/data/x/sodium.jar"],
                    "hashes": { "sha1": "deadbeef", "sha512": "long" },
                    "fileSize": 123456,
                    "env": { "client": "required", "server": "optional" }
                },
                {
                    "path": "mods/server-only.jar",
                    "downloads": ["https://example.com/server-only.jar"],
                    "hashes": { "sha1": "cafe" },
                    "env": { "client": "unsupported", "server": "required" }
                },
                {
                    "path": "config/no-hash.toml",
                    "downloads": ["https://example.com/cfg.toml"]
                }
            ]
        }"#;

        let index: MrpackIndex = serde_json::from_str(sample).unwrap();
        assert_eq!(index.format_version, 1);
        assert_eq!(index.name, "My Modpack");
        assert_eq!(index.dependencies.get("minecraft").map(String::as_str), Some("1.20.1"));
        assert_eq!(
            index.dependencies.get("fabric-loader").map(String::as_str),
            Some("0.15.7")
        );
        assert_eq!(index.files.len(), 3);

        // 文件 0:client required → 受支持;sha1 / size / downloads 正确。
        let f0 = &index.files[0];
        assert_eq!(f0.path, "mods/sodium.jar");
        assert!(f0.client_supported());
        assert_eq!(f0.downloads.first().map(String::as_str), Some("https://cdn.modrinth.com/data/x/sodium.jar"));
        assert_eq!(f0.hashes.sha1.as_deref(), Some("deadbeef"));
        assert_eq!(f0.file_size, Some(123456));

        // 文件 1:client unsupported → 应被跳过。
        assert!(!index.files[1].client_supported());

        // 文件 2:无 env、无 hashes → 受支持(缺省)、sha1 None。
        let f2 = &index.files[2];
        assert!(f2.client_supported());
        assert!(f2.hashes.sha1.is_none());
        assert_eq!(f2.file_size, None);
    }

    #[test]
    fn parse_mrpack_index_minimal() {
        // 仅含 minecraft 依赖的最小索引也应解析成功。
        let sample = r#"{"formatVersion":1,"dependencies":{"minecraft":"1.21"}}"#;
        let index: MrpackIndex = serde_json::from_str(sample).unwrap();
        assert_eq!(index.dependencies.get("minecraft").map(String::as_str), Some("1.21"));
        assert!(index.files.is_empty());
        assert!(index.name.is_empty());
    }

    #[test]
    fn exported_dependencies_include_fabric_loader_from_proxy_chain() {
        let root = TempRoot::new("export-fabric-deps");
        root.add_version_json("1.20.1", r#"{"id":"1.20.1","libraries":[]}"#);
        root.add_version_json(
            "fabric-loader-0.15.7-1.20.1",
            r#"{
                "id":"fabric-loader-0.15.7-1.20.1",
                "inheritsFrom":"1.20.1",
                "libraries":[{"name":"net.fabricmc:fabric-loader:0.15.7"}]
            }"#,
        );
        root.add_version_json(
            "friends-pack",
            r#"{"id":"friends-pack","inheritsFrom":"fabric-loader-0.15.7-1.20.1"}"#,
        );

        let inst = Instance::new("friends-pack", root.path.clone());
        let deps = mrpack_dependencies(&inst, "1.20.1");

        assert_eq!(deps.get("minecraft").map(String::as_str), Some("1.20.1"));
        assert_eq!(deps.get("fabric-loader").map(String::as_str), Some("0.15.7"));
    }

    #[test]
    fn exported_dependencies_strip_forge_minecraft_prefix() {
        let root = TempRoot::new("export-forge-deps");
        root.add_version_json("1.20.1", r#"{"id":"1.20.1","libraries":[]}"#);
        root.add_version_json(
            "1.20.1-forge-47.2.0",
            r#"{
                "id":"1.20.1-forge-47.2.0",
                "inheritsFrom":"1.20.1",
                "libraries":[{"name":"net.minecraftforge:forge:1.20.1-47.2.0"}]
            }"#,
        );

        let inst = Instance::new("1.20.1-forge-47.2.0", root.path.clone());
        let deps = mrpack_dependencies(&inst, "1.20.1");

        assert_eq!(deps.get("forge").map(String::as_str), Some("47.2.0"));
    }

    #[test]
    fn loader_dependency_fallback_parses_profile_ids() {
        assert_eq!(
            detect_loader_dependency_from_id("fabric-loader-0.15.7-1.20.1", "1.20.1"),
            Some(("fabric-loader", "0.15.7".to_string()))
        );
        assert_eq!(
            detect_loader_dependency_from_id("1.20.1-forge-47.2.0", "1.20.1"),
            Some(("forge", "47.2.0".to_string()))
        );
        assert_eq!(
            detect_loader_dependency_from_id("neoforge-20.4.237", "1.20.4"),
            Some(("neoforge", "20.4.237".to_string()))
        );
    }

    // ---- export / import overrides 往返 ----

    #[test]
    fn export_then_reimport_overrides_roundtrip() {
        let root = TempRoot::new("export");
        // 造实例:mods + config,带嵌套目录。
        let inst = Instance::new("1.20.1", root.path.clone());
        let game_dir = inst.game_dir();
        fs::create_dir_all(game_dir.join("mods")).unwrap();
        fs::write(game_dir.join("mods/cool.jar"), b"COOLMOD").unwrap();
        fs::create_dir_all(game_dir.join("config/sub")).unwrap();
        fs::write(game_dir.join("config/sub/opts.toml"), b"key=1").unwrap();
        // 给实例起个名,验证写进索引。
        let mut cfg = inst.load_config().unwrap();
        cfg.name = Some("Exported Pack".to_string());
        inst.save_config(&cfg).unwrap();

        // 导出。
        let dest = root.path.join("out.mrpack");
        export_mrpack(&inst, "1.20.1", &dest).unwrap();
        assert!(dest.is_file());

        // 打开导出的 zip,校验索引与 overrides 内容。
        let f = fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();

        // 索引存在且内容正确。
        let index: MrpackIndex = {
            let mut e = archive.by_name(MRPACK_INDEX_ENTRY).unwrap();
            let mut s = String::new();
            e.read_to_string(&mut s).unwrap();
            serde_json::from_str(&s).unwrap()
        };
        assert_eq!(index.format_version, 1);
        assert_eq!(index.name, "Exported Pack");
        assert_eq!(index.dependencies.get("minecraft").map(String::as_str), Some("1.20.1"));
        assert!(index.files.is_empty(), "本地导出 files 应为空,全部走 overrides");

        // overrides 条目存在。
        let mut cool = archive.by_name("overrides/mods/cool.jar").unwrap();
        let mut buf = Vec::new();
        cool.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"COOLMOD");
        drop(cool);
        let mut opts = archive.by_name("overrides/config/sub/opts.toml").unwrap();
        let mut buf2 = Vec::new();
        opts.read_to_end(&mut buf2).unwrap();
        assert_eq!(buf2, b"key=1");
        drop(opts);

        // 再用 extract_overrides 解到一个新 game_dir,验证 import 侧解压路径正确。
        let target = root.path.join("reimport-game-dir");
        fs::create_dir_all(&target).unwrap();
        extract_overrides(&mut archive, &target).unwrap();
        assert_eq!(fs::read(target.join("mods/cool.jar")).unwrap(), b"COOLMOD");
        assert_eq!(fs::read(target.join("config/sub/opts.toml")).unwrap(), b"key=1");
    }

    #[test]
    fn client_overrides_take_precedence() {
        let root = TempRoot::new("client-ov");
        let dest = root.path.join("ov.mrpack");

        // 手工造一个含 overrides 与 client-overrides 同名文件的 zip。
        {
            let out = fs::File::create(&dest).unwrap();
            let mut zw = zip::ZipWriter::new(out);
            let opt = zip::write::SimpleFileOptions::default();
            zw.start_file(MRPACK_INDEX_ENTRY, opt).unwrap();
            zw.write_all(br#"{"formatVersion":1,"dependencies":{"minecraft":"1.20.1"}}"#).unwrap();
            zw.start_file("overrides/config/shared.txt", opt).unwrap();
            zw.write_all(b"GENERIC").unwrap();
            zw.start_file("client-overrides/config/shared.txt", opt).unwrap();
            zw.write_all(b"CLIENT").unwrap();
            zw.finish().unwrap();
        }

        let f = fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let target = root.path.join("game-dir");
        fs::create_dir_all(&target).unwrap();
        extract_overrides(&mut archive, &target).unwrap();

        // client-overrides 第二遍写入,应覆盖 overrides。
        assert_eq!(
            fs::read_to_string(target.join("config/shared.txt")).unwrap(),
            "CLIENT",
            "client-overrides 应覆盖通用 overrides"
        );
    }

    #[test]
    fn extract_overrides_blocks_zip_slip() {
        let root = TempRoot::new("slip");
        let dest = root.path.join("evil.mrpack");
        {
            let out = fs::File::create(&dest).unwrap();
            let mut zw = zip::ZipWriter::new(out);
            let opt = zip::write::SimpleFileOptions::default();
            // 一个试图越权写到父目录的条目。
            zw.start_file("overrides/../../escaped.txt", opt).unwrap();
            zw.write_all(b"PWNED").unwrap();
            zw.finish().unwrap();
        }
        let f = fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let target = root.path.join("game-dir");
        fs::create_dir_all(&target).unwrap();
        let err = extract_overrides(&mut archive, &target).unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "zip-slip 应被拒绝");
    }
}
