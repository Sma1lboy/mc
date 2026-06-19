//! 资源包 / 光影 / 数据包的本地数据层。
//!
//! 这三类资源在磁盘上的形态高度一致——都是放在实例某个固定子目录下的
//! `*.zip`(数据包还可能是解压后的目录),通过 `.disabled` 后缀来开关——
//! 因此用同一套代码统一处理,差异只在"放到哪个目录"与"对应平台资源类型"。
//!
//! 设计要点:
//! - **无状态扫描**:列表每次从磁盘实时读取(对齐 [`crate::instance`] 的整体风格),
//!   不缓存,避免内存与磁盘不一致。
//! - **启用约定**:沿用各启动器/游戏本身的通行做法——以 `.disabled` 结尾即禁用。
//!   切换启用态只是文件 rename(不动内容),零拷贝且可逆。
//! - **删除走回收站**:[`delete_pack`] 优先用 `trash` 移入系统回收站(可被用户找回),
//!   仅在回收站不可用(如无 GUI 环境)时回退到不可逆删除。
//! - **安全文件名**:所有按 `file_name` 定位的操作都拒绝路径分隔符 / `..`,
//!   防止越权操作实例目录之外的文件。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, IoResultExt, Result};
use crate::instance::Instance;
use crate::modplatform::{ProjectVersion, ResourceKind};

/// 禁用文件名后缀。以此结尾的资源被游戏视为关闭状态。
const DISABLED_SUFFIX: &str = ".disabled";

/// 本数据层管理的三类"包"资源。
///
/// 与 [`ResourceKind`] 的区别:`PackKind` 只覆盖"放在实例目录、按文件开关"的三类,
/// 不含 Mod / Modpack(它们的管理逻辑不同);并额外携带"落在哪个子目录"的信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackKind {
    ResourcePack,
    Shader,
    Datapack,
}

impl PackKind {
    /// 该类资源在实例下的安装目录。
    pub fn dir(&self, inst: &Instance) -> PathBuf {
        match self {
            PackKind::ResourcePack => inst.resourcepacks_dir(),
            PackKind::Shader => inst.shaderpacks_dir(),
            PackKind::Datapack => inst.datapacks_dir(),
        }
    }

    /// 映射到内容平台的资源类型,供搜索 / 取版本时使用。
    pub fn to_resource_kind(&self) -> ResourceKind {
        match self {
            PackKind::ResourcePack => ResourceKind::ResourcePack,
            PackKind::Shader => ResourceKind::Shader,
            PackKind::Datapack => ResourceKind::Datapack,
        }
    }

    /// 给前端展示用的稳定字符串标签(也写进 [`PackInfo::kind`])。
    fn label(&self) -> &'static str {
        match self {
            PackKind::ResourcePack => "resourcepack",
            PackKind::Shader => "shader",
            PackKind::Datapack => "datapack",
        }
    }
}

/// 一个已安装包的列表视图。直接派生 `Serialize` 以便经 Tauri command 回传前端。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackInfo {
    /// 磁盘上的文件(或目录)名,含可能的 `.disabled` 后缀——
    /// 这是后续 enable/disable/delete 操作的定位键。
    pub file_name: String,
    /// 是否启用(= 不以 `.disabled` 结尾)。
    pub enabled: bool,
    /// 资源类型标签(`resourcepack` / `shader` / `datapack`),来自 [`PackKind::label`]。
    pub kind: &'static str,
    /// 文件大小(字节);目录形态的数据包为 0(不递归求和,避免大目录扫描开销)。
    pub size: u64,
    /// 资源包 `pack.mcmeta` 里的描述文本;读取失败 / 非资源包时为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// 判断文件名是否为我们识别的包文件:`.zip` / `.jar`(忽略大小写),
/// 允许带 `.disabled` 后缀。数据包的目录形态由调用处单独处理。
fn is_pack_archive(name: &str) -> bool {
    // 先剥掉 .disabled 再看真实扩展名,使 "foo.zip.disabled" 也被识别。
    let base = name.strip_suffix(DISABLED_SUFFIX).unwrap_or(name);
    let lower = base.to_ascii_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".jar")
}

/// 从启用态文件名推断是否启用。
fn is_enabled(name: &str) -> bool {
    !name.ends_with(DISABLED_SUFFIX)
}

/// 校验 `file_name` 是单一路径段(不含分隔符、不是 `.`/`..`),防止路径穿越。
/// 通过后返回 `dir.join(file_name)` 的安全绝对路径。
fn resolve_in_dir(dir: &std::path::Path, file_name: &str) -> Result<PathBuf> {
    if file_name.is_empty()
        || file_name == "."
        || file_name == ".."
        || file_name.contains('/')
        || file_name.contains('\\')
    {
        return Err(CoreError::other(format!("非法包文件名: {file_name}")));
    }
    Ok(dir.join(file_name))
}

/// 尝试从一个资源包 zip 里读取 `pack.mcmeta` 的 `pack.description` 字段。
///
/// 任何失败(不是 zip、无该条目、json 不含该字段)都返回 `None`——
/// 描述是纯展示性的可选信息,绝不应让列表因它失败。
/// description 可能是字符串,也可能是富文本(对象 / 数组),后者统一序列化为紧凑 json 文本。
fn read_resourcepack_description(path: &std::path::Path) -> Option<String> {
    use std::io::Read;

    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name("pack.mcmeta").ok()?;

    let mut raw = String::new();
    entry.read_to_string(&mut raw).ok()?;

    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let desc = value.get("pack")?.get("description")?;
    match desc {
        serde_json::Value::String(s) => Some(s.clone()),
        // 富文本描述(对象/数组):退化为紧凑 json,至少把内容透出给上层。
        other => Some(other.to_string()),
    }
}

/// 列出某实例下指定类型的全部包。
///
/// 扫描规则:
/// - 普通文件:扩展名为 `.zip`/`.jar`(允许 `.disabled` 后缀)才计入。
/// - 目录:仅数据包计入(原版支持解压目录形态的数据包);其余类型忽略目录。
/// - 目录不存在 / 读取失败:返回空列表(不报错——尚未创建是正常状态)。
///
/// 仅资源包会尝试读取 `pack.mcmeta` 描述(其它类型无此约定),失败静默忽略。
/// 结果按 `file_name` 字典序稳定排序,保证展示顺序确定。
pub fn list_packs(inst: &Instance, kind: PackKind) -> Vec<PackInfo> {
    let dir = kind.dir(inst);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut out: Vec<PackInfo> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };

        let is_dir = path.is_dir();
        if is_dir {
            // 只有数据包接受目录形态;且跳过点目录(如残留的临时目录)。
            if kind != PackKind::Datapack || file_name.starts_with('.') {
                continue;
            }
        } else if !is_pack_archive(&file_name) {
            continue;
        }

        // 目录无意义的"文件大小";普通文件取 metadata.len()。
        let size = if is_dir {
            0
        } else {
            std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
        };

        // 仅资源包、且是启用的 zip 文件时才尝试读 pack.mcmeta(开销最小化)。
        let description = if kind == PackKind::ResourcePack && !is_dir {
            read_resourcepack_description(&path)
        } else {
            None
        };

        out.push(PackInfo {
            enabled: is_enabled(&file_name),
            kind: kind.label(),
            size,
            description,
            file_name,
        });
    }

    out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    out
}

/// 切换某个包的启用态。通过增删 `.disabled` 后缀(rename)实现,不触碰内容。
///
/// - `enabled == true`:若当前是 `xxx.disabled` 则改名回 `xxx`;已启用则空操作。
/// - `enabled == false`:若当前未禁用则改名为 `xxx.disabled`;已禁用则空操作。
///
/// 目标名已存在时返回错误(避免静默覆盖另一个同名包)。
pub fn set_pack_enabled(
    inst: &Instance,
    kind: PackKind,
    file_name: &str,
    enabled: bool,
) -> Result<()> {
    let dir = kind.dir(inst);
    let src = resolve_in_dir(&dir, file_name)?;

    // 计算目标文件名:启用即去后缀,禁用即加后缀。
    let target_name = if enabled {
        match file_name.strip_suffix(DISABLED_SUFFIX) {
            Some(base) => base.to_string(),
            None => return Ok(()), // 已是启用态,无需改名。
        }
    } else {
        if file_name.ends_with(DISABLED_SUFFIX) {
            return Ok(()); // 已是禁用态。
        }
        format!("{file_name}{DISABLED_SUFFIX}")
    };

    let dst = dir.join(&target_name);
    if dst.exists() {
        return Err(CoreError::other(format!("目标已存在,无法切换: {target_name}")));
    }
    std::fs::rename(&src, &dst).with_path(&src)
}

/// 删除一个包(优先移入系统回收站,可被用户找回)。
///
/// `trash::delete` 在无 GUI / 不支持回收站的环境会失败,此时回退到不可逆删除
/// (文件用 `remove_file`,目录用 `remove_dir_all`)以保证操作最终生效。
pub fn delete_pack(inst: &Instance, kind: PackKind, file_name: &str) -> Result<()> {
    let dir = kind.dir(inst);
    let path = resolve_in_dir(&dir, file_name)?;

    // 不存在视为已删除(幂等),不报错。
    if !path.exists() {
        return Ok(());
    }

    // 首选回收站:可恢复,符合"删除资源"这类用户可逆操作的预期。
    if trash::delete(&path).is_ok() {
        return Ok(());
    }

    // 回退:不可逆删除。区分文件 / 目录。
    if path.is_dir() {
        std::fs::remove_dir_all(&path).with_path(&path)
    } else {
        std::fs::remove_file(&path).with_path(&path)
    }
}

/// 从一个平台版本安装包到对应目录,返回落盘的文件名。
///
/// 取该版本的主文件([`ProjectVersion::primary_file`]),下载到
/// `kind.dir(inst)/<filename>`(下载器会自动建父目录 + sha1 校验)。
/// 版本不含任何文件时返回错误。
pub async fn install_pack_version(
    inst: &Instance,
    dl: &crate::download::Downloader,
    kind: PackKind,
    v: &ProjectVersion,
) -> Result<String> {
    let file = v
        .primary_file()
        .ok_or_else(|| CoreError::other(format!("版本 {} 没有可下载文件", v.id)))?;

    let dir = kind.dir(inst);
    // primary file 的 filename 由平台给出,理论可信;仍按单一路径段校验防御。
    let path = resolve_in_dir(&dir, &file.filename)?;

    let item = crate::download::DownloadItem {
        url: file.url.clone(),
        path: path.clone(),
        sha1: file.sha1.clone(),
        size: file.size,
    };
    dl.download_one(&item).await?;

    Ok(file.filename.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// 在临时目录里搭一个最小 game root,测试结束自动清理。
    /// 结构:`<root>/versions/<id>/resourcepacks/...`(game_dir == version_dir)。
    struct TempInst {
        root: PathBuf,
        inst: Instance,
    }

    impl TempInst {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("mc-core-packs-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let inst = Instance::new("1.20.1", root.clone());
            // 预建资源包目录,模拟已有实例。
            fs::create_dir_all(inst.resourcepacks_dir()).unwrap();
            Self { root, inst }
        }
    }

    impl Drop for TempInst {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn lists_with_enabled_flag_and_count() {
        let t = TempInst::new("list");
        let rp = t.inst.resourcepacks_dir();
        // 一个启用的、一个禁用的。
        fs::write(rp.join("Faithful.zip"), b"PK\x03\x04not-a-real-zip").unwrap();
        fs::write(rp.join("OldPack.zip.disabled"), b"PK\x03\x04not-a-real-zip").unwrap();
        // 一个无关文件(txt),应被忽略。
        fs::write(rp.join("readme.txt"), b"ignore me").unwrap();

        let packs = list_packs(&t.inst, PackKind::ResourcePack);
        assert_eq!(packs.len(), 2, "只应列出两个 zip 包,忽略 txt");

        let faithful = packs.iter().find(|p| p.file_name == "Faithful.zip").unwrap();
        assert!(faithful.enabled, "无 .disabled 后缀应为启用");
        assert_eq!(faithful.kind, "resourcepack");
        assert!(faithful.size > 0);

        let old = packs.iter().find(|p| p.file_name == "OldPack.zip.disabled").unwrap();
        assert!(!old.enabled, ".disabled 后缀应为禁用");
    }

    #[test]
    fn set_enabled_renames_off_and_on() {
        let t = TempInst::new("toggle");
        let rp = t.inst.resourcepacks_dir();
        fs::write(rp.join("Pack.zip"), b"data").unwrap();

        // 禁用:应改名为 Pack.zip.disabled。
        set_pack_enabled(&t.inst, PackKind::ResourcePack, "Pack.zip", false).unwrap();
        assert!(!rp.join("Pack.zip").exists());
        assert!(rp.join("Pack.zip.disabled").exists());

        let after_disable = list_packs(&t.inst, PackKind::ResourcePack);
        assert_eq!(after_disable.len(), 1);
        assert!(!after_disable[0].enabled);

        // 再启用:应改回 Pack.zip。
        set_pack_enabled(&t.inst, PackKind::ResourcePack, "Pack.zip.disabled", true).unwrap();
        assert!(rp.join("Pack.zip").exists());
        assert!(!rp.join("Pack.zip.disabled").exists());

        let after_enable = list_packs(&t.inst, PackKind::ResourcePack);
        assert!(after_enable[0].enabled);
    }

    #[test]
    fn set_enabled_is_idempotent() {
        let t = TempInst::new("idem");
        let rp = t.inst.resourcepacks_dir();
        fs::write(rp.join("A.zip"), b"x").unwrap();

        // 已启用再启用:空操作,文件不变。
        set_pack_enabled(&t.inst, PackKind::ResourcePack, "A.zip", true).unwrap();
        assert!(rp.join("A.zip").exists());
    }

    #[test]
    fn rejects_path_traversal_filename() {
        let t = TempInst::new("traversal");
        let err = set_pack_enabled(&t.inst, PackKind::ResourcePack, "../evil.zip", true);
        assert!(err.is_err(), "含 .. 的文件名必须被拒绝");
        let err2 = set_pack_enabled(&t.inst, PackKind::ResourcePack, "sub/evil.zip", false);
        assert!(err2.is_err(), "含分隔符的文件名必须被拒绝");
    }

    #[test]
    fn datapack_lists_directories() {
        let t = TempInst::new("datapack-dir");
        let dp = t.inst.datapacks_dir();
        fs::create_dir_all(dp.join("MyDatapack")).unwrap();
        fs::write(dp.join("vanilla-tweaks.zip"), b"PK").unwrap();

        let packs = list_packs(&t.inst, PackKind::Datapack);
        assert_eq!(packs.len(), 2, "数据包应同时列出目录形态与 zip 形态");
        assert!(packs.iter().any(|p| p.file_name == "MyDatapack"));
        assert!(packs.iter().any(|p| p.file_name == "vanilla-tweaks.zip"));
    }

    #[test]
    fn missing_dir_lists_empty() {
        let t = TempInst::new("missing");
        // shaderpacks 目录未创建。
        let packs = list_packs(&t.inst, PackKind::Shader);
        assert!(packs.is_empty());
    }

    #[test]
    fn reads_resourcepack_description() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let t = TempInst::new("mcmeta");
        let rp = t.inst.resourcepacks_dir();
        let zip_path = rp.join("Described.zip");

        // 写一个真实的 zip,内含 pack.mcmeta。
        let file = fs::File::create(&zip_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        zw.start_file("pack.mcmeta", SimpleFileOptions::default()).unwrap();
        zw.write_all(br#"{"pack":{"pack_format":15,"description":"A cool pack"}}"#)
            .unwrap();
        zw.finish().unwrap();

        let packs = list_packs(&t.inst, PackKind::ResourcePack);
        let described = packs.iter().find(|p| p.file_name == "Described.zip").unwrap();
        assert_eq!(described.description.as_deref(), Some("A cool pack"));
    }

    #[test]
    fn pack_kind_maps_to_dir_and_resource_kind() {
        let t = TempInst::new("kindmap");
        assert_eq!(PackKind::ResourcePack.dir(&t.inst), t.inst.resourcepacks_dir());
        assert_eq!(PackKind::Shader.dir(&t.inst), t.inst.shaderpacks_dir());
        assert_eq!(PackKind::Datapack.dir(&t.inst), t.inst.datapacks_dir());

        assert_eq!(PackKind::ResourcePack.to_resource_kind(), ResourceKind::ResourcePack);
        assert_eq!(PackKind::Shader.to_resource_kind(), ResourceKind::Shader);
        assert_eq!(PackKind::Datapack.to_resource_kind(), ResourceKind::Datapack);
    }

    #[test]
    fn delete_removes_file() {
        let t = TempInst::new("delete");
        let rp = t.inst.resourcepacks_dir();
        let target = rp.join("Trash.zip");
        fs::write(&target, b"bye").unwrap();

        delete_pack(&t.inst, PackKind::ResourcePack, "Trash.zip").unwrap();
        assert!(!target.exists(), "删除后文件应不在原位(回收站或硬删均可)");

        // 重复删除不存在的文件应幂等成功。
        delete_pack(&t.inst, PackKind::ResourcePack, "Trash.zip").unwrap();
    }
}
