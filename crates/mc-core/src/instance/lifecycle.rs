//! 实例生命周期(复制 / 删除)与 Modrinth 整合包(`.mrpack`)的导入 / 导出。
//!
//! 在"版本即实例"模型下,一个实例就是一个 `versions/<id>/` 目录:版本 json
//! (`versions/<id>/<id>.json`)、可选的客户端 jar(`versions/<id>/<id>.jar`)、
//! `instance.json`,以及运行时游戏数据(mods/saves/config/resourcepacks…)全部
//! 平铺在该目录下(game_dir == version_dir)。本模块围绕这一布局提供四个操作:
//!
//! - [`copy_instance`]:整目录复制为新 id,并把版本 json/jar 的内部 id 一并改名,
//!   使复制出的实例自洽可启动。
//! - [`delete_instance`][]:把整个实例目录移入回收站(失败回退到不可逆删除)。
//! - [`import_mrpack`]:解析 Modrinth `.mrpack`(本质是 zip),安装其声明的原版
//!   Minecraft、覆盖 `overrides/`、下载 `files[]` 到实例目录。loader 的安装不在
//!   本批做(见函数文档),由上层在导入后另行调用 `loader::install_*`。
//! - [`export_mrpack`][]:把实例的本地游戏数据(mods/config/resourcepacks…)打成一个
//!   最小可用的 `.mrpack`,所有本地文件都放进 `overrides/` 下(无远程 url 时的兜底)。
//!
//! 设计要点:
//! - **zip-slip 防护**:解压 `files[]` 与 `overrides/` 里的相对路径前都用
//!   [`crate::fs::safe_join`] 收口到实例目录内,拒绝 `../` 越权。
//! - **env 容错**:`files[].env.client == "unsupported"` 的文件(纯服务端文件)在
//!   客户端导入时跳过,不下载。
//! - **保真落盘**:版本 json 用 `serde_json::Value` 只改 `id` 字段后写出,其余字段
//!   原样保留(避免重新序列化丢失未建模字段)。

use std::path::Path;

use tokio::sync::watch;

use mc_types::{LoaderKind, Progress};

use crate::download::Downloader;
use crate::error::{CoreError, IoResultExt, Result};
use crate::instance::{Instance, InstanceConfig};
use crate::paths::GamePaths;

// ===========================================================================
// 从零建实例
// ===========================================================================

/// 从零创建一个实例:装核心(原版或 + loader)→ 写一个 `inheritsFrom` 核心的命名版本
/// json → 写实例配置(展示名 + 默认内存等)。返回新实例 id(展示名的 slug,冲突自动加序号)。
///
/// 与整合包导入**共用** [`crate::loader::install_core`] 装核心,故 Forge/Fabric/Quilt/
/// NeoForge 都支持;`loader == None` 即纯原版实例。建好后 mods/资源包/设置等照常按实例管理
/// (`install_mod` / `list_mods` / `list_packs` / `InstanceConfig`)。
#[allow(clippy::too_many_arguments)]
pub async fn create_instance(
    dl: &Downloader,
    paths: &GamePaths,
    name: &str,
    mc_version: &str,
    loader: Option<(LoaderKind, String)>,
    // 新实例默认内存(MB)与 Java 路径,通常来自全局设置(default_memory_mb / java_path)。
    default_memory_mb: u32,
    default_java_path: Option<String>,
    progress: Option<watch::Sender<Progress>>,
) -> Result<String> {
    // 1. 装核心,拿到实例应继承的版本 id。
    let core_id =
        crate::loader::install_core(dl, paths, mc_version, loader.as_ref(), progress).await?;

    // 2. 唯一实例 id(展示名 slug;与已存在版本目录冲突则加序号)。
    let instance_id = unique_instance_id(paths, name);

    // 3. 让实例本身成为可启动版本:写最小 {id, inheritsFrom: core_id}。
    //    unique_instance_id 已避开所有已存在目录,故 instance_id != core_id 必成立;留判防御。
    if instance_id != core_id {
        let json = serde_json::json!({ "id": instance_id, "inheritsFrom": core_id });
        let raw = serde_json::to_string_pretty(&json)
            .map_err(|e| CoreError::Parse { what: "instance version json".into(), source: e })?;
        crate::fs::write_atomic(&paths.version_json(&instance_id), raw.as_bytes())?;
    }

    // 4. 写实例配置:展示名 + 全局默认(内存/Java 取自全局设置,其余取 InstanceConfig::default)。
    let inst = Instance::new(&instance_id, paths.root().to_path_buf());
    inst.save_config(&InstanceConfig {
        name: Some(name.to_string()),
        memory_mb: if default_memory_mb > 0 { default_memory_mb } else { InstanceConfig::default().memory_mb },
        java_path: default_java_path.filter(|p| !p.is_empty()),
        ..InstanceConfig::default()
    })?;

    Ok(instance_id)
}

/// 加入领域时建一个**薄存根**实例:只写 `instance.json`(展示名 + realm 绑定),不装核心。
/// 返回新实例 id。核心(版本 + loader + mods)留给「开始同步」([`materialize_core`] + 领域同步)。
/// 这样「加入」是即时的轻动作,重活由用户显式触发。
pub fn create_realm_shell(
    paths: &GamePaths,
    name: &str,
    realm: crate::types::RealmRef,
    default_memory_mb: u32,
    default_java_path: Option<String>,
) -> Result<String> {
    let instance_id = unique_instance_id(paths, name);
    let inst = Instance::new(&instance_id, paths.root().to_path_buf());
    inst.save_config(&InstanceConfig {
        name: Some(name.to_string()),
        memory_mb: if default_memory_mb > 0 { default_memory_mb } else { InstanceConfig::default().memory_mb },
        java_path: default_java_path.filter(|p| !p.is_empty()),
        realm: Some(realm),
        ..InstanceConfig::default()
    })?;
    Ok(instance_id)
}

/// 给一个已有(薄存根)实例装核心:装原版 + 可选 loader,并把实例自身写成可启动版本
/// json(`{ id, inheritsFrom: core_id }`)。**幂等**:已装(版本 json 已存在)直接返回。
/// 供「开始同步」首段调用(装核心 → 再下 mods)。
pub async fn materialize_core(
    dl: &Downloader,
    paths: &GamePaths,
    instance_id: &str,
    mc_version: &str,
    loader: Option<(LoaderKind, String)>,
    progress: Option<watch::Sender<Progress>>,
) -> Result<()> {
    if paths.version_json(instance_id).exists() {
        return Ok(()); // 已装核心,幂等。
    }
    let core_id = crate::loader::install_core(dl, paths, mc_version, loader.as_ref(), progress).await?;
    if instance_id != core_id {
        let json = serde_json::json!({ "id": instance_id, "inheritsFrom": core_id });
        let raw = serde_json::to_string_pretty(&json)
            .map_err(|e| CoreError::Parse { what: "instance version json".into(), source: e })?;
        crate::fs::write_atomic(&paths.version_json(instance_id), raw.as_bytes())?;
    }
    Ok(())
}

/// 在一个已有实例上写入 / 清除领域绑定:host「分享为领域」时写入(role=owner),
/// 退出 / 解散时清除(传 `None`)。保留其余配置不变。
pub fn set_instance_realm(
    paths: &GamePaths,
    instance_id: &str,
    realm: Option<crate::types::RealmRef>,
) -> Result<()> {
    let inst = Instance::new(instance_id, paths.root().to_path_buf());
    let mut cfg = InstanceConfig::load(&inst.config_path()).unwrap_or_default();
    cfg.realm = realm;
    inst.save_config(&cfg)
}

/// 由展示名推一个文件系统安全、且当前 root 下唯一的实例 id(= 目录名)。
fn unique_instance_id(paths: &GamePaths, name: &str) -> String {
    let base = slugify_instance_id(name);
    crate::fs::unique_name(&base, |cand| paths.version_dir(cand).exists())
}

/// 把展示名化简成目录名(保留 unicode,空回退 `instance`)。规则与世界文件夹共用
/// [`crate::fs::slugify`] 这一份 owner。
fn slugify_instance_id(name: &str) -> String {
    crate::fs::slugify(name, "instance")
}

// ===========================================================================
// 给已有实例加 loader(核心)
// ===========================================================================

/// 版本 json 里我们需要读的两个字段(id / inheritsFrom)。和 `instance/mod.rs::VersionHead`
/// 同形,但本模块自留一份以免跨模块暴露私有类型。
#[derive(serde::Deserialize)]
struct InstanceHead {
    #[serde(rename = "inheritsFrom")]
    inherits_from: Option<String>,
}

/// 给一个**已存在**的实例追加 / 切换 mod 加载器(core)。
///
/// 「版本即实例」模型下实例是个薄存根(`{ id, inheritsFrom: core_id }`),加 loader 就是
/// 把它重指向「原版 + loader」核心。返回**之后应使用的实例 id** —— 多数情况下与传入
/// `instance_id` 相同,但退化情形(见下)会返回一个新 id。
///
/// 两种情形:
/// - **常规**:实例已带 `inheritsFrom`(或 id 本就不等于裸 mc 版本,即它是个存根而非
///   原版目录本身)。直接装核心,再把存根 json 重写为 `{ id, inheritsFrom: core_id }`,id 不变。
/// - **退化**:实例没有 `inheritsFrom` **且** `instance_id == mc_version`(实例目录就是那份
///   裸原版,如 id `"1.20.1"`)。若原地重指向会造成「loader 核心 inheritsFrom 1.20.1 ==
///   本目录」的自环,故先把本目录改名到一个新 id(连同内部 `<id>.json` / `<id>.jar` 一并改名),
///   腾出 `mc_version` 这个名字;再装核心([`crate::loader::install_core`] 见缺原版会重建之),
///   最后把改名后的实例存根重指向 loader 核心。返回新 id。
///
/// `loader == Vanilla`(或解析不出 loader)会被拒绝 —— 「加原版」无意义。
pub async fn add_loader(
    dl: &Downloader,
    paths: &GamePaths,
    instance_id: &str,
    mc_version: &str,
    loader: (LoaderKind, String),
    progress: Option<watch::Sender<Progress>>,
) -> Result<String> {
    // 0. 校验:必须是真正的 loader。
    if loader.0 == LoaderKind::Vanilla {
        return Err(CoreError::other("不能给实例添加「原版」加载器"));
    }

    // 1. 读实例存根的 head(只取 id / inheritsFrom)。实例必须已存在。
    let inst_json = paths.version_json(instance_id);
    let raw = std::fs::read_to_string(&inst_json).with_path(&inst_json)?;
    let head: InstanceHead = serde_json::from_str(&raw)
        .map_err(|e| CoreError::Parse { what: "instance version json".into(), source: e })?;

    // 2. 区分退化情形:无 inheritsFrom 且实例目录本身就是裸原版(id == mc_version)。
    let is_degenerate = head.inherits_from.is_none() && instance_id == mc_version;

    if is_degenerate {
        // 2a. 先把裸原版目录改名到一个新 id,腾出 mc_version 这个名字给 install_core 重建。
        let new_id = unique_loader_instance_id(paths, instance_id, loader.0);
        rename_instance_dir(paths, instance_id, &new_id)?;

        // 2b. 装核心:原版 mc_version 此刻已不存在,install_core 会重新拉回它并装好 loader。
        let core_id = crate::loader::install_core(dl, paths, mc_version, Some(&loader), progress).await?;

        // 2c. 把改名后的实例存根重指向 loader 核心。
        relink_instance_stub(paths, &new_id, &core_id)?;
        Ok(new_id)
    } else {
        // 常规:装核心后原地把存根重指向它,id 不变。
        let core_id = crate::loader::install_core(dl, paths, mc_version, Some(&loader), progress).await?;
        relink_instance_stub(paths, instance_id, &core_id)?;
        Ok(instance_id.to_string())
    }
}

/// 为退化情形挑一个唯一的新实例 id:基底 `{instance_id}-{loader}`(loader 小写),
/// 已存在则 `-2` / `-3`…(复用 [`unique_instance_id`] 的避让逻辑)。
fn unique_loader_instance_id(paths: &GamePaths, instance_id: &str, loader: LoaderKind) -> String {
    let base = format!("{instance_id}-{}", loader.as_str().to_ascii_lowercase());
    unique_instance_id(paths, &base)
}

/// 把版本目录 `versions/<new_id>/` 里仍沿用 `old_id` 命名的 id-绑定文件改名到 `new_id`:
/// 版本 json(`<old_id>.json` → `<new_id>.json`,内部 `"id"` 字段同步改写)与客户端 jar
/// (`<old_id>.jar` → `<new_id>.jar`,内容与 id 无关只改文件名)。复制([`copy_instance`])
/// 与移动([`rename_instance_dir`])的收尾都走它,保证两条路径对「id-绑定文件」的处理一致——
/// 这是「给一个版本目录里的 id-绑定文件改名」的唯一 owner。
fn reid_version_files(paths: &GamePaths, old_id: &str, new_id: &str) -> Result<()> {
    let dir = paths.version_dir(new_id);

    // 版本 json:改写内部 id,落到新名文件,删掉沿用旧名的那份。
    let old_json = dir.join(format!("{old_id}.json"));
    let new_json = paths.version_json(new_id);
    if old_json.is_file() {
        let raw = std::fs::read_to_string(&old_json).with_path(&old_json)?;
        let rewritten = rewrite_version_id(&raw, new_id)?;
        crate::fs::write_atomic(&new_json, rewritten.as_bytes())?;
        if old_json != new_json {
            std::fs::remove_file(&old_json).with_path(&old_json)?;
        }
    }

    // 客户端 jar:仅改名(内容与 id 无关)。
    let old_jar = dir.join(format!("{old_id}.jar"));
    let new_jar = paths.version_jar(new_id);
    if old_jar.is_file() && old_jar != new_jar {
        std::fs::rename(&old_jar, &new_jar).with_path(&old_jar)?;
    }
    Ok(())
}

/// 把版本目录 `versions/<old_id>/` 整体改名为 `versions/<new_id>/`,并把目录内随 id 命名的
/// `<old_id>.json` / `<old_id>.jar` 一并改名为 `<new_id>.*`(json 内部 `id` 字段同步改写)。
///
/// 与 [`copy_instance`] 的差异:这是**移动**(原 id 目录消失),用于退化情形腾出 mc_version 名字;
/// 其余游戏数据(mods/saves/instance.json/icon)随目录一并迁移,无需逐类处理。要求 `new_id`
/// 目录此前不存在(由调用方经 [`unique_loader_instance_id`] 保证)。
fn rename_instance_dir(paths: &GamePaths, old_id: &str, new_id: &str) -> Result<()> {
    let old_dir = paths.version_dir(old_id);
    let new_dir = paths.version_dir(new_id);
    if new_dir.exists() {
        return Err(CoreError::other(format!("目标实例目录 {new_id} 已存在,无法改名")));
    }
    // 1) 整目录改名。
    std::fs::rename(&old_dir, &new_dir).with_path(&old_dir)?;

    // 2) 目录内随旧 id 命名的 json / jar 改名并改写内部 id(与 copy_instance 共用 owner)。
    reid_version_files(paths, old_id, new_id)
}

/// 把实例存根版本 json 重写为最小的 `{ id, inheritsFrom: core_id }`(原子写)。
///
/// 实例是薄存根,除 id / inheritsFrom 外不承载版本元数据(库/参数都由继承链上的 core 提供),
/// 故直接覆盖即可,无需保留其它字段。
fn relink_instance_stub(paths: &GamePaths, instance_id: &str, core_id: &str) -> Result<()> {
    let json = serde_json::json!({ "id": instance_id, "inheritsFrom": core_id });
    let raw = serde_json::to_string_pretty(&json)
        .map_err(|e| CoreError::Parse { what: "instance version json".into(), source: e })?;
    crate::fs::write_atomic(&paths.version_json(instance_id), raw.as_bytes())
}

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

    // 2) 复制出的目录里跟 id 绑定的 json / jar 改名并改写内部 id(与 rename_instance_dir
    //    共用 owner)。复制后磁盘上是 versions/<new_id>/<src_id>.*。
    reid_version_files(paths, src_id, new_id)
}

/// 按展示名把实例 `src_id` 复制为一个新实例:由 `new_name` 推出唯一安全的目录 id,
/// 整目录复制(见 [`copy_instance`]),再把复制出的 `instance.json` 的 `name` 设为
/// `new_name`(以便列表/管理弹窗显示新名,而非沿用旧实例名)。返回新实例 id。
///
/// 这是 UI「复制实例」的入口:调用方只给一个人类可读的新名,id 的唯一化与安全化在此处理。
pub fn copy_instance_named(paths: &GamePaths, src_id: &str, new_name: &str) -> Result<String> {
    let new_id = unique_instance_id(paths, new_name);
    copy_instance(paths, src_id, &new_id)?;

    // 复制带过来的 instance.json 仍是旧名,改写成用户给的新名;name 为空时回退用 id。
    let inst = Instance::new(new_id.clone(), paths.root().to_path_buf());
    let mut config = inst.load_config().unwrap_or_default();
    let trimmed = new_name.trim();
    config.name = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
    inst.save_config(&config)?;

    Ok(new_id)
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
// 导入 / 导出
// ===========================================================================
//
// `modrinth.index.json` 的字段级模型已统一到 [`crate::modpack::formats::mrpack`];
// **导入的格式知识 + 解压 / 下载 / 装核心管线已上移到 [`crate::modpack::import`]**
// (可插拔架构),本模块的 [`import_mrpack`] 退化为薄包装。导出仍在本模块(把实例本地
// 游戏数据自包含打成 `.mrpack`)。

/// `modrinth.index.json` 在 `.mrpack` 内的固定路径(导出写索引用)。
const MRPACK_INDEX_ENTRY: &str = "modrinth.index.json";
/// 通用 overrides 目录前缀(导出把本地数据铺进它)。
const OVERRIDES_PREFIX: &str = "overrides/";

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modpack::formats::mrpack::MrpackIndex;
    use std::fs;
    use std::io::{Read, Write};
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
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    // ---- create_instance helpers ----

    #[test]
    fn slugify_instance_id_cleans_and_keeps_unicode() {
        assert_eq!(slugify_instance_id("My Pack 1.20"), "My-Pack-1.20");
        assert_eq!(slugify_instance_id("  weird/\\:name? "), "weird-name");
        assert_eq!(slugify_instance_id("我的整合包"), "我的整合包"); // 保留中文
        assert_eq!(slugify_instance_id("///"), "instance"); // 空结果回退
        assert_eq!(slugify_instance_id("a   b"), "a-b"); // 空白归一
    }

    #[test]
    fn unique_instance_id_suffixes_on_collision() {
        let root = TempRoot::new("unique");
        let paths = root.paths();
        assert_eq!(unique_instance_id(&paths, "Pack"), "Pack");
        fs::create_dir_all(paths.version_dir("Pack")).unwrap();
        assert_eq!(unique_instance_id(&paths, "Pack"), "Pack-2");
        fs::create_dir_all(paths.version_dir("Pack-2")).unwrap();
        assert_eq!(unique_instance_id(&paths, "Pack"), "Pack-3");
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
    fn copy_instance_named_uniquifies_id_and_rewrites_name() {
        let root = TempRoot::new("copy-named");
        let paths = root.paths();

        fs::create_dir_all(paths.version_dir("1.20.1")).unwrap();
        fs::write(paths.version_json("1.20.1"), r#"{"id":"1.20.1"}"#).unwrap();
        fs::write(paths.version_dir("1.20.1").join("instance.json"), r#"{"name":"Source"}"#).unwrap();

        // 首次复制:id 由名字 slug 化。
        let id1 = copy_instance_named(&paths, "1.20.1", "我的副本").unwrap();
        assert_eq!(id1, "我的副本");
        let cfg1 = Instance::new(id1.clone(), paths.root().to_path_buf()).load_config().unwrap();
        assert_eq!(cfg1.name.as_deref(), Some("我的副本"), "新实例名应改写为给定名");

        // 再复制同名:id 自动加后缀避免目录冲突,name 仍为给定名。
        let id2 = copy_instance_named(&paths, "1.20.1", "我的副本").unwrap();
        assert_eq!(id2, "我的副本-2");
        let cfg2 = Instance::new(id2, paths.root().to_path_buf()).load_config().unwrap();
        assert_eq!(cfg2.name.as_deref(), Some("我的副本"));
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

    // ---- add_loader: re-id + relink (network-free parts) ----

    #[test]
    fn add_loader_validation_rejects_vanilla() {
        // 给实例加「原版」无意义,应被拒绝。整段 add_loader 走异步 + 真实下载器,
        // 这里只验证早返回的校验分支(不触发任何网络)。
        let root = TempRoot::new("add-loader-vanilla");
        let paths = root.paths();
        fs::create_dir_all(paths.version_dir("1.20.1")).unwrap();
        fs::write(paths.version_json("1.20.1"), r#"{"id":"1.20.1"}"#).unwrap();

        let dl = crate::download::Downloader::new(1).unwrap();
        let err = futures::executor::block_on(add_loader(
            &dl,
            &paths,
            "1.20.1",
            "1.20.1",
            (LoaderKind::Vanilla, String::new()),
            None,
        ))
        .unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "vanilla 应被拒绝");
    }

    #[test]
    fn add_loader_unique_id_for_degenerate() {
        // 退化情形的新 id 命名:{instance_id}-{loader}(小写),冲突加序号。
        let root = TempRoot::new("add-loader-uid");
        let paths = root.paths();
        assert_eq!(unique_loader_instance_id(&paths, "1.20.1", LoaderKind::Fabric), "1.20.1-fabric");

        // 该名已被占用时加序号。
        fs::create_dir_all(paths.version_dir("1.20.1-fabric")).unwrap();
        assert_eq!(unique_loader_instance_id(&paths, "1.20.1", LoaderKind::Fabric), "1.20.1-fabric-2");

        // NeoForge 小写化。
        assert_eq!(
            unique_loader_instance_id(&paths, "1.21", LoaderKind::NeoForge),
            "1.21-neoforge"
        );
    }

    #[test]
    fn relink_instance_stub_overwrites_with_inherits() {
        // (i) 常规情形的核心动作:把存根 json 重写为 {id, inheritsFrom: core_id}。
        let root = TempRoot::new("relink");
        let paths = root.paths();
        // 一个已带 inheritsFrom 的薄实例存根(继承原版)。
        fs::create_dir_all(paths.version_dir("my-pack")).unwrap();
        fs::write(
            paths.version_json("my-pack"),
            r#"{"id":"my-pack","inheritsFrom":"1.20.1"}"#,
        )
        .unwrap();

        // 重指向 loader 核心(模拟 install_core 返回的 core_id)。
        relink_instance_stub(&paths, "my-pack", "fabric-loader-0.15.7-1.20.1").unwrap();

        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(paths.version_json("my-pack")).unwrap()).unwrap();
        assert_eq!(v["id"], "my-pack", "实例 id 不变");
        assert_eq!(
            v["inheritsFrom"], "fabric-loader-0.15.7-1.20.1",
            "应重指向 loader 核心"
        );
        // 存根只保留这两个键(薄存根语义)。
        assert_eq!(v.as_object().unwrap().len(), 2);
    }

    #[test]
    fn rename_instance_dir_moves_and_rewrites_id() {
        // (ii) 退化情形的核心动作:把裸原版目录整体改名到新 id,内部 json/jar 一并改名,
        // 改名后原 id(== mc_version)的目录消失,可供 install_core 重建原版。
        let root = TempRoot::new("reid");
        let paths = root.paths();

        // 造一个「实例目录就是裸原版」的退化实例:id "1.20.1",带 jar、mod、icon、instance.json。
        let old_dir = paths.version_dir("1.20.1");
        fs::create_dir_all(&old_dir).unwrap();
        fs::write(
            paths.version_json("1.20.1"),
            r#"{"id":"1.20.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )
        .unwrap();
        fs::write(paths.version_jar("1.20.1"), b"FAKEJAR").unwrap();
        fs::create_dir_all(old_dir.join("mods")).unwrap();
        fs::write(old_dir.join("mods/sodium.jar"), b"MODBYTES").unwrap();
        fs::write(old_dir.join("icon.png"), b"\x89PNGicon").unwrap();
        fs::write(old_dir.join("instance.json"), r#"{"name":"My World","memory_mb":4096}"#).unwrap();

        rename_instance_dir(&paths, "1.20.1", "1.20.1-fabric").unwrap();

        // 原 id 目录已不存在 → mc_version "1.20.1" 这个名字腾出,可被 install_core 重建。
        assert!(!old_dir.exists(), "原裸原版目录应已改名消失");

        // 新目录:json 改名 + 内部 id 改写,其余字段保留。
        let new_json = paths.version_json("1.20.1-fabric");
        assert!(new_json.is_file(), "应生成 1.20.1-fabric.json");
        let new_dir = paths.version_dir("1.20.1-fabric");
        assert!(!new_dir.join("1.20.1.json").exists(), "旧名 json 应被删除");
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&new_json).unwrap()).unwrap();
        assert_eq!(v["id"], "1.20.1-fabric");
        assert_eq!(v["mainClass"], "net.minecraft.client.main.Main");
        assert_eq!(v["type"], "release");

        // jar 随 id 改名。
        assert!(paths.version_jar("1.20.1-fabric").is_file(), "jar 应改名为 1.20.1-fabric.jar");
        assert!(!new_dir.join("1.20.1.jar").exists());

        // 游戏数据 / icon / instance.json 随目录迁移。
        assert_eq!(fs::read(new_dir.join("mods/sodium.jar")).unwrap(), b"MODBYTES");
        assert_eq!(fs::read(new_dir.join("icon.png")).unwrap(), b"\x89PNGicon");
        assert!(new_dir.join("instance.json").is_file());

        // 模拟 add_loader 退化分支后续:install_core 重建 "1.20.1" 原版 + 把新实例重指向 loader 核心。
        fs::create_dir_all(&old_dir).unwrap();
        fs::write(paths.version_json("1.20.1"), r#"{"id":"1.20.1","type":"release"}"#).unwrap();
        relink_instance_stub(&paths, "1.20.1-fabric", "fabric-loader-0.15.7-1.20.1").unwrap();

        // 原 mc_version 原版重新可解析。
        assert!(paths.version_json("1.20.1").is_file(), "重建后的原版应可解析");
        // 新实例已重指向 loader 核心,id 为新 id。
        let relinked: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(new_json).unwrap()).unwrap();
        assert_eq!(relinked["id"], "1.20.1-fabric");
        assert_eq!(relinked["inheritsFrom"], "fabric-loader-0.15.7-1.20.1");
    }

    #[test]
    fn rename_instance_dir_rejects_existing_target() {
        let root = TempRoot::new("reid-exists");
        let paths = root.paths();
        fs::create_dir_all(paths.version_dir("1.20.1")).unwrap();
        fs::write(paths.version_json("1.20.1"), r#"{"id":"1.20.1"}"#).unwrap();
        fs::create_dir_all(paths.version_dir("1.20.1-fabric")).unwrap();

        let err = rename_instance_dir(&paths, "1.20.1", "1.20.1-fabric").unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "目标已存在应返回 Other 错误");
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
                    "path": "config/only-sha512.toml",
                    "downloads": ["https://example.com/cfg.toml"],
                    "hashes": { "sha512": "onlybig" }
                }
            ]
        }"#;

        let index: MrpackIndex = serde_json::from_str(sample).unwrap();
        assert_eq!(index.format_version, 1);
        assert_eq!(index.name, "My Modpack");
        assert_eq!(index.dependencies.minecraft.as_deref(), Some("1.20.1"));
        assert_eq!(index.dependencies.fabric_loader.as_deref(), Some("0.15.7"));
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

        // 文件 2:无 env → 受支持(缺省)、sha1 None。
        let f2 = &index.files[2];
        assert!(f2.client_supported());
        assert!(f2.hashes.sha1.is_none());
        assert_eq!(f2.file_size, None);
    }

    #[test]
    fn parse_mrpack_index_minimal() {
        // 仅含必需字段(game/name/dependencies.minecraft)的最小索引也应解析成功。
        let sample =
            r#"{"formatVersion":1,"game":"minecraft","name":"Mini","dependencies":{"minecraft":"1.21"}}"#;
        let index: MrpackIndex = serde_json::from_str(sample).unwrap();
        assert_eq!(index.dependencies.minecraft.as_deref(), Some("1.21"));
        assert!(index.files.is_empty());
        assert_eq!(index.name, "Mini");
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
        assert_eq!(index.dependencies.minecraft.as_deref(), Some("1.20.1"));
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

        // 再用导入侧的归档解压器(ZipArchiveIndex::extract_prefix)解到一个新 game_dir,
        // 验证导出的 overrides 能被导入管线正确还原(往返闭环;override 铺设细节的
        // 单测在 modpack::import::archive::tests)。
        drop(archive);
        let target = root.path.join("reimport-game-dir");
        fs::create_dir_all(&target).unwrap();
        let mut idx = crate::modpack::import::archive::ZipArchiveIndex::open(&dest).unwrap();
        idx.extract_prefix(OVERRIDES_PREFIX, &target).unwrap();
        assert_eq!(fs::read(target.join("mods/cool.jar")).unwrap(), b"COOLMOD");
        assert_eq!(fs::read(target.join("config/sub/opts.toml")).unwrap(), b"key=1");
    }

    #[test]
    fn export_writes_index_and_overrides_prefix() {
        // 导出产物的结构契约:索引为 formatVersion=1 且 files 为空,本地数据落在
        // overrides/ 前缀下(client-overrides 覆盖语义的单测在 import::archive::tests)。
        let root = TempRoot::new("export-struct");
        let inst = Instance::new("1.20.1", root.path.clone());
        let game_dir = inst.game_dir();
        fs::create_dir_all(game_dir.join("config")).unwrap();
        fs::write(game_dir.join("config/shared.txt"), b"GENERIC").unwrap();

        let dest = root.path.join("ov.mrpack");
        export_mrpack(&inst, "1.20.1", &dest).unwrap();

        let f = fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let mut shared = archive.by_name("overrides/config/shared.txt").unwrap();
        let mut buf = String::new();
        shared.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "GENERIC");
    }

    #[test]
    fn import_archive_extract_blocks_zip_slip() {
        // 导入侧解压同样拒绝 zip-slip(与导出无关的安全闸,经 ZipArchiveIndex)。
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
        let target = root.path.join("game-dir");
        fs::create_dir_all(&target).unwrap();
        let mut idx = crate::modpack::import::archive::ZipArchiveIndex::open(&dest).unwrap();
        let err = idx.extract_prefix(OVERRIDES_PREFIX, &target).unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "zip-slip 应被拒绝");
    }
}
