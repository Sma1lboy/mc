//! 本地 Mod 管理 —— 扫描实例 `mods/` 目录、读取每个 jar 的元数据、启停与删除。
//!
//! 设计要点:
//! - 一个 mod 文件可能处于"启用"或"停用"两态。沿用 MultiMC/Prism/HMCL 的约定:
//!   停用的 mod 文件名末尾追加 `.disabled`(即 `foo.jar` ↔ `foo.jar.disabled`),
//!   loader 只加载 `*.jar`,从而无需移动文件即可启停。
//! - 元数据来源各 loader 不同:
//!   · Fabric:`fabric.mod.json`(JSON)
//!   · Quilt :`quilt.mod.json`(JSON,字段在 `quilt_loader` 下)
//!   · Forge / NeoForge:`META-INF/mods.toml` / `META-INF/neoforge.mods.toml`(TOML 文本)
//!   为不引入额外依赖,Forge 系的 TOML 用一个**轻量、容错**的手写解析器提取所需字段。
//! - **健壮性优先**:解析单个 jar 出任何错误(打不开、不是 zip、字段缺失、JSON 损坏)
//!   都不得 panic,也不能中断对其它 jar 的扫描 —— 该 jar 退化为"仅文件名"信息。
//! - `ModInfo` 实现 `Serialize` 以便直接经 IPC/HTTP 输出给上层 UI。

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, IoResultExt, Result};
use crate::instance::Instance;

/// 单个本地 mod 的元数据视图。字段尽量贴近 UI 列表展示所需。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ModInfo {
    /// 磁盘上的实际文件名(含 `.disabled` 后缀,如有)。用作启停/删除的稳定标识。
    pub file_name: String,
    /// 是否启用 = 文件名不以 `.disabled` 结尾。
    pub enabled: bool,
    /// 展示名;读不到元数据时回退为去掉扩展名的文件名。
    pub name: String,
    /// 版本号(部分 mod 用占位符如 `${version}`,原样保留)。
    pub version: Option<String>,
    /// mod 内部 id(fabric `id` / quilt `id` / forge `modId`)。
    pub mod_id: Option<String>,
    /// loader 家族:`fabric` / `quilt` / `forge` / `neoforge` / `unknown`。
    pub loader: String,
    /// 作者列表(各 loader 格式差异较大,这里统一拍平成字符串数组)。
    pub authors: Vec<String>,
    /// 描述(通常为一句话)。
    pub description: Option<String>,
    /// 文件字节大小。
    pub size: u64,
}

/// 停用文件的统一后缀。
const DISABLED_SUFFIX: &str = ".disabled";

/// 列出实例 `mods/` 目录下的所有 mod(包含已停用的)。
///
/// 扫描规则:接受文件名以 `.jar` 或 `.jar.disabled` 结尾的普通文件。
/// 对每个文件尝试读取元数据;读取/解析失败的文件会退化为"仅文件名"条目,
/// **不会**被丢弃也**不会** panic。结果按 `name` 不区分大小写排序,保证展示顺序稳定。
///
/// 若 `mods/` 目录不存在(实例从未装过 mod),返回空列表。
pub fn list_mods(inst: &Instance) -> Vec<ModInfo> {
    let mods_dir = inst.mods_dir();

    let entries = match std::fs::read_dir(&mods_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut out: Vec<ModInfo> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) => s.to_string(),
            None => continue, // 非 UTF-8 文件名,跳过(无法稳定标识)。
        };

        // 只接受 *.jar 与 *.jar.disabled。
        let enabled = if file_name.ends_with(DISABLED_SUFFIX) {
            // 去掉 .disabled 后必须仍是 .jar 才算 mod 文件(排除 foo.txt.disabled 之类)。
            if !strip_disabled(&file_name).ends_with(".jar") {
                continue;
            }
            false
        } else if file_name.ends_with(".jar") {
            true
        } else {
            continue;
        };

        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        // 解析元数据;任何失败都退化为基于文件名的兜底信息。
        let mut info = read_mod_metadata(&path).unwrap_or_else(|| fallback_info(&file_name));
        info.file_name = file_name;
        info.enabled = enabled;
        info.size = size;

        out.push(info);
    }

    // 不区分大小写按展示名排序;同名再按文件名兜底,保证确定性。
    out.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    out
}

/// 启用/停用一个 mod:在 `<name>.jar` 与 `<name>.jar.disabled` 之间重命名。
///
/// `file_name` 为 [`list_mods`] 返回的当前文件名(可带或不带 `.disabled`)。
/// 若目标态已满足(已是想要的后缀),则为 no-op,直接返回 `Ok`。
pub fn set_mod_enabled(inst: &Instance, file_name: &str, enabled: bool) -> Result<()> {
    // file_name 来自前端,校验为单一路径段防穿越(rename 出 mods/ 会破坏其它目录)。
    if !crate::fs::is_safe_segment(file_name) {
        return Err(crate::error::CoreError::other(format!("非法 mod 文件名: {file_name}")));
    }
    let mods_dir = inst.mods_dir();

    // 以"去掉 .disabled 的基名"为锚,推导当前路径与目标路径,避免依赖传入后缀是否正确。
    let base = strip_disabled(file_name).to_string();
    let enabled_path = mods_dir.join(&base);
    let disabled_path = mods_dir.join(format!("{base}{DISABLED_SUFFIX}"));

    let (src, dst) = if enabled {
        (disabled_path, enabled_path)
    } else {
        (enabled_path, disabled_path)
    };

    // 已处于目标态:目标存在且源不存在 → 无需操作。
    if dst.exists() && !src.exists() {
        return Ok(());
    }

    std::fs::rename(&src, &dst).with_path(&src)
}

/// 删除一个 mod 文件。优先移入系统回收站(可恢复);若回收站不可用(如某些 Linux
/// 无 trash 环境、CI),回退到不可逆的 [`std::fs::remove_file`]。
///
/// `file_name` 可带或不带 `.disabled`;实际删除磁盘上存在的那一个。
pub fn delete_mod(inst: &Instance, file_name: &str) -> Result<()> {
    // file_name 来自前端,校验为单一路径段防穿越(删除可逃出 mods/ 误删其它文件)。
    if !crate::fs::is_safe_segment(file_name) {
        return Err(crate::error::CoreError::other(format!("非法 mod 文件名: {file_name}")));
    }
    let mods_dir = inst.mods_dir();

    // 定位真实存在的文件:传入名 → 基名(.jar) → 停用名(.jar.disabled)。
    let base = strip_disabled(file_name).to_string();
    let candidates = [
        mods_dir.join(file_name),
        mods_dir.join(&base),
        mods_dir.join(format!("{base}{DISABLED_SUFFIX}")),
    ];

    let target = candidates.into_iter().find(|p| p.exists());
    let target = match target {
        Some(p) => p,
        // 文件已不存在:视作删除成功(幂等),避免上层因竞态报错。
        None => return Ok(()),
    };

    // 删除走统一 owner:优先回收站,失败回退硬删除(此处恒为文件)。
    crate::fs::trash_or_delete(&target)
}

/// 读取单个 jar 的内部 `mod_id`(读不出 / 无该字段时返回 `None`)。
/// 用于「装新版自动清掉同一 mod 的旧 jar」时按 mod_id 配对。
pub fn read_mod_id(path: &Path) -> Option<String> {
    read_mod_metadata(path).and_then(|m| m.mod_id)
}

/// 装好 `keep_file_name` 这个新版本后,清理 `mods/` 里**同一个 mod(相同 `mod_id`)
/// 但文件名不同**的旧 jar —— 否则两个版本会声明同一 `modId`,导致游戏启动直接崩溃
/// (duplicate mod id)。返回被清理掉的旧文件名列表。
///
/// 安全约束:读不出新文件的 `mod_id` 时**不删除任何东西**(无法可靠判定归属);
/// 旧文件优先移入回收站(可找回),回收站不可用才硬删。已停用(`.disabled`)的同
/// mod 旧版同样清理 —— 留着它也会在用户重新启用时再次冲突。
pub fn remove_superseded(inst: &Instance, keep_file_name: &str) -> Result<Vec<String>> {
    let mods_dir = inst.mods_dir();

    let keep_id = match read_mod_id(&mods_dir.join(keep_file_name)) {
        Some(id) => id,
        None => return Ok(Vec::new()),
    };

    let entries = match std::fs::read_dir(&mods_dir) {
        Ok(e) => e,
        Err(_) => return Ok(Vec::new()),
    };

    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if name == keep_file_name {
            continue;
        }
        // 只看 mod jar(含 .disabled),其它文件忽略。
        let is_mod_jar = name.ends_with(".jar")
            || (name.ends_with(DISABLED_SUFFIX) && strip_disabled(&name).ends_with(".jar"));
        if !is_mod_jar {
            continue;
        }
        if read_mod_id(&path).as_deref() == Some(keep_id.as_str()) {
            crate::fs::trash_or_delete(&path)?;
            removed.push(name);
        }
    }
    Ok(removed)
}

/// 把一个本地 `.jar` 拖拽导入实例 `mods/` 目录,返回落盘文件名。
///
/// 校验:源必须存在、文件名是 `.jar`(忽略大小写)。重名直接覆盖——用户主动拖入即视为
/// 替换意图。文件名取源的 basename(不含路径分量),天然不含路径穿越。
pub fn import_local_mod(inst: &Instance, source: &Path) -> Result<String> {
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CoreError::other("无效的文件名"))?
        .to_string();
    if !name.to_ascii_lowercase().ends_with(".jar") {
        return Err(CoreError::other("只支持拖入 .jar mod 文件"));
    }
    let dir = inst.mods_dir();
    std::fs::create_dir_all(&dir).with_path(&dir)?;
    std::fs::copy(source, dir.join(&name)).with_path(source)?;
    Ok(name)
}

mod meta;
#[cfg(test)]
mod tests;

use meta::*;
