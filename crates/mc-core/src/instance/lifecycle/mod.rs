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
        relink_instance_stub(paths, &instance_id, &core_id)?;
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
        relink_instance_stub(paths, instance_id, &core_id)?;
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
    let head: crate::version::VersionHead = serde_json::from_str(&raw)
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

/// 把实例存根版本 json 写成最小的 `{ id, inheritsFrom: core_id }`(原子写)。**新建存根与
/// 重指向已有存根是同一动作**(write_atomic 直接覆盖),所以创建实例 / 装核心 / 整合包导入
/// 都走这同一个 owner——「写实例存根 json」此前被逐字内联/复制了四份。
///
/// 实例是薄存根,除 id / inheritsFrom 外不承载版本元数据(库/参数都由继承链上的 core 提供),
/// 故直接覆盖即可,无需保留其它字段。
pub(crate) fn relink_instance_stub(paths: &GamePaths, instance_id: &str, core_id: &str) -> Result<()> {
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
    // 删除走统一 owner:优先回收站,失败回退硬删除(实例目录恒为目录)。
    crate::fs::trash_or_delete(&dir)
}

mod mrpack;
#[cfg(test)]
mod tests;

pub use mrpack::*;
