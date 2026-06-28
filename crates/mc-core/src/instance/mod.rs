//! 实例管理 —— "版本即实例"模型。
//!
//! 不像 Prism 那样把实例放在独立的 `instances/<name>/.minecraft/` 里,本启动器
//! 直接把每个 game root 下的 `versions/<id>/` 当作一个实例(贴近官启/HMCL/PCL 的
//! 目录布局,见 `docs/07-directory-model-portability.md` 与 `docs/modules/instance.md`)。
//!
//! 一个目录 `versions/<id>/` 被视为**用户实例**,当且仅当其中存在版本 json
//! (`versions/<id>/<id>.json`)**且没有其它目录 `inheritsFrom` 指向它** —— 后者是被
//! 继承的依赖核心(原版 / loader profile),不是用户实例,枚举时隐藏(见 [`list_instances`])。
//! 实例的可覆盖设置存放在同目录的 `instance.json`(见 [`config`])。本模块只负责
//! "枚举/读写已存在的实例目录",实际的版本/库/资源下载由 launch/meta 层负责。

pub mod config;
pub mod install_mod;
pub mod lifecycle;
pub mod mods;
pub mod packs;
pub mod screenshots;
pub mod servers;
pub mod update;
pub mod world;

pub use config::{InstanceConfig, InstanceSource};
pub use install_mod::{
    install_mod, install_mod_version, install_mod_version_with_deps, InstallReport,
};
pub use mods::{list_mods, ModInfo};
pub use packs::{install_pack, list_packs, PackInfo, PackKind};
pub use screenshots::{list_screenshots, read_screenshot, ScreenshotInfo};
pub use servers::{add_server, read_servers, SavedServer};
pub use update::{
    apply_mod_update, check_all_updates, check_mod_updates, InstanceUpdateInfo, ModUpdate,
};
pub use world::{import_world_zip, list_worlds, WorldInfo};

use std::path::{Path, PathBuf};

use mc_types::{InstanceSummary, LoaderKind};

use crate::error::{CoreError, IoResultExt, Result};
use crate::paths::GamePaths;
use crate::version::VersionHead;

/// `instance.json` 在实例目录下的固定文件名。
const INSTANCE_CONFIG_FILE: &str = "instance.json";

/// 指向某个 game root 下的单个实例(`versions/<id>/`)。
///
/// `Instance` 只持有定位信息(实例 id 与所属 game root),不缓存任何配置内容 ——
/// 配置每次按需从磁盘读取,避免内存与磁盘状态不一致。
#[derive(Debug, Clone)]
pub struct Instance {
    id: String,
    root: PathBuf,
}

impl Instance {
    /// 在 `root`(game root)下绑定 id 为 `id` 的实例。不做存在性检查 ——
    /// 调用方若需要可先用 [`Instance::dir`] / [`GamePaths::version_json`] 判断。
    pub fn new(id: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        Self { id: id.into(), root: root.into() }
    }

    /// 该实例所属 game root 的目录布局视图。
    pub fn paths(&self) -> GamePaths {
        GamePaths::new(self.root.clone())
    }

    /// 实例(= 版本)id。
    pub fn version_id(&self) -> &str {
        &self.id
    }

    /// 实例目录:`<root>/versions/<id>/`。
    pub fn dir(&self) -> PathBuf {
        self.paths().version_dir(&self.id)
    }

    /// `instance.json` 的完整路径。
    pub fn config_path(&self) -> PathBuf {
        self.dir().join(INSTANCE_CONFIG_FILE)
    }

    /// 该实例的游戏运行目录(`--gameDir`)。当前模型下等于实例目录,
    /// saves/mods/resourcepacks 等运行时数据都在此(版本隔离)。
    pub fn game_dir(&self) -> PathBuf {
        self.dir()
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.game_dir().join("mods")
    }

    pub fn saves_dir(&self) -> PathBuf {
        self.game_dir().join("saves")
    }

    pub fn resourcepacks_dir(&self) -> PathBuf {
        self.game_dir().join("resourcepacks")
    }

    pub fn shaderpacks_dir(&self) -> PathBuf {
        self.game_dir().join("shaderpacks")
    }

    pub fn datapacks_dir(&self) -> PathBuf {
        self.game_dir().join("datapacks")
    }

    pub fn screenshots_dir(&self) -> PathBuf {
        self.game_dir().join("screenshots")
    }

    /// 实例图标路径:`versions/<id>/icon.png`(与导入器写入位置、[`detect_icon`] 探测位置一致)。
    pub fn icon_path(&self) -> PathBuf {
        self.game_dir().join("icon.png")
    }

    /// 把任意图片设为该实例的图标:校验后拷贝到 [`Instance::icon_path`]。
    ///
    /// 防御:源必须存在、是受支持的图片(按魔数嗅探)、且不超过 1 MiB(与 [`detect_icon`]
    /// 的内联上限一致,避免设了却显示不出)。内容按原样拷贝(不重编码),文件名固定 `icon.png`
    /// 但真实格式由 [`detect_icon`] 嗅探决定 mime。
    pub fn set_icon(&self, source: &Path) -> Result<()> {
        let bytes = std::fs::read(source).with_path(source)?;
        self.set_icon_bytes(&bytes)
    }

    /// [`set_icon`] 的字节版:校验(非空 / ≤1 MiB / 魔数可识别)后写入 [`Instance::icon_path`]。
    /// 用于「从 URL 下载的整合包图标」直接落盘,不必先写临时文件再读回。
    pub fn set_icon_bytes(&self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Err(CoreError::other("图标文件为空"));
        }
        if bytes.len() > 1024 * 1024 {
            return Err(CoreError::other("图标过大(上限 1 MiB)"));
        }
        if sniff_image_mime_opt(bytes).is_none() {
            return Err(CoreError::other("无法识别的图片格式(支持 png/jpg/gif/bmp/webp)"));
        }
        let dest = self.icon_path();
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).with_path(parent)?;
        }
        std::fs::write(&dest, bytes).with_path(&dest)
    }

    /// 该实例是否已有本地图标文件(`versions/<id>/icon.png`)。补齐图标前先判这个,避免重复下载。
    pub fn has_icon(&self) -> bool {
        self.icon_path().exists()
    }

    /// 读取该实例的配置;文件不存在时返回 [`InstanceConfig::default`]。
    pub fn load_config(&self) -> Result<InstanceConfig> {
        InstanceConfig::load(&self.config_path())
    }

    /// 将配置写入该实例的 `instance.json`(自动创建目录)。
    pub fn save_config(&self, config: &InstanceConfig) -> Result<()> {
        config.save(&self.config_path())
    }
}

/// 由实例 id 与可选的 `inheritsFrom` 推断 loader 家族。
///
/// loader 版本 json 通常带有提示性 id / `inheritsFrom`(如 `fabric-loader-0.15-1.20.1`、
/// `1.20.1-forge-47.2.0`),这里做轻量子串匹配即可,无需解析整份 json。
/// 顺序上先匹配更具体的 `neoforge`(它也包含子串 `forge`)。
fn infer_loader(id: &str, inherits_from: Option<&str>) -> LoaderKind {
    let mut hay = id.to_ascii_lowercase();
    if let Some(parent) = inherits_from {
        hay.push(' ');
        hay.push_str(&parent.to_ascii_lowercase());
    }

    if hay.contains("neoforge") {
        LoaderKind::NeoForge
    } else if hay.contains("forge") {
        LoaderKind::Forge
    } else if hay.contains("fabric") {
        LoaderKind::Fabric
    } else if hay.contains("quilt") {
        LoaderKind::Quilt
    } else if hay.contains("optifine") {
        LoaderKind::OptiFine
    } else if hay.contains("liteloader") {
        LoaderKind::LiteLoader
    } else {
        LoaderKind::Vanilla
    }
}

/// 探测实例目录下的 `icon.png`,有则读出编码为 `data:` URL,供前端 `<img>` 直接用。
///
/// 约定与导入器一致:实例图标固定落在 `versions/<id>/icon.png`(本模型下
/// `game_dir == version_dir`)。内联成 data URL 而非返回路径,避免在两套布局里都
/// 去配 Tauri asset 协议 / 作用域;图标通常只有几十 KB,代价可忽略。
///
/// 任何失败(无文件 / 读取错误 / 空文件 / 过大)一律返回 `None` —— 图标是纯展示性的,
/// 缺省时 UI 会用首字母占位兜底,绝不应让列表因图标失败。设 1 MiB 上限防止把异常大的
/// 图塞进每次 list 的 json。
fn detect_icon(dir: &Path) -> Option<String> {
    let bytes = std::fs::read(dir.join("icon.png")).ok()?;
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return None;
    }
    Some(format!("data:{};base64,{}", sniff_image_mime(&bytes), base64_encode(&bytes)))
}

/// 从文件头的魔数判断图片 mime。图标文件名固定为 `icon.png`,但用户可能拖入 jpg/webp 等,
/// 故按内容嗅探 mime,保证 data URL 声明正确、各 webview 都能渲染;无法识别时退回 png
/// (探测既有 icon.png 时从宽,坏数据也至多渲染失败,不影响列表)。
pub(crate) fn sniff_image_mime(bytes: &[u8]) -> &'static str {
    sniff_image_mime_opt(bytes).unwrap_or("image/png")
}

/// 严格版嗅探:仅当魔数匹配已知图片格式时返回 `Some(mime)`,否则 `None`。
/// 用于 [`Instance::set_icon`] 在写盘前拒绝非图片文件。
fn sniff_image_mime_opt(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// 标准 base64 编码(带 `=` 填充)。手写以免为"把图标内联进 data URL"这一处引入依赖。
pub(crate) fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { ALPHABET[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// 取目录的最后修改时间(epoch millis),失败返回 0。
fn dir_mtime_millis(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 从某实例的 `instance.json` 读 `last_played`。这里的"上次游玩"语义由 launch 层
/// 在启动时写入 config;若 config 不存在或未记录,则回退到实例目录的 mtime。
///
/// 注:[`InstanceConfig`] 当前不含 `last_played` 字段,故统一回退到目录 mtime;
/// 该 helper 独立出来便于将来在 config 增加该字段时只改一处。
fn last_played_for(dir: &Path) -> u64 {
    dir_mtime_millis(dir)
}

/// 沿 `inheritsFrom` 链解析到根(无 inheritsFrom 的原版版本),返回其 id 作为基础 mc 版本。
///
/// 普通 loader 实例只继承一层(loader → 原版);整合包实例可能多层
/// (实例 → loader → 原版),后者若只取一层会把 loader id 当成 mc 版本。带深度上限防御
/// 异常循环;父 json 读不到时停在当前已知最深的 id(它就是要找的根)。
fn resolve_base_mc_version(paths: &GamePaths, id: &str, inherits: Option<&str>) -> String {
    // 复用 version::walk_inherits 的守护遍历(环检测 + 深度上限),只遍历 id 链。
    // 容错策略放在闭包里:父 json 读不到/解析失败 → 视为到根(parent = None),
    // 遍历停在当前层。leaf 用调用方已知的 `inherits` 以免重读 leaf json。
    let start_inherits = inherits.map(|s| s.to_string());
    let chain = crate::version::walk_inherits(id, |cur| {
        let parent = if cur == id {
            start_inherits.clone()
        } else {
            std::fs::read_to_string(paths.version_json(cur))
                .ok()
                .and_then(|raw| VersionHead::parse(&raw))
                .and_then(|h| h.inherits_from)
        };
        Ok::<_, crate::error::CoreError>(crate::version::InheritNode {
            payload: cur.to_string(),
            parent,
        })
    });
    // 走到的最深 id 即基础 mc 版本;环/异常时回退到 leaf id。
    match chain {
        Ok(ids) => ids.into_iter().last().unwrap_or_else(|| id.to_string()),
        Err(_) => id.to_string(),
    }
}

/// 从实例继承链里解析出**真实**的 loader 版本。fabric/quilt 的 loader profile id 形如
/// `fabric-loader-0.19.3-26.2`(= `<family>-loader-<loaderver>-<mcver>`),取中间的
/// `0.19.3`。供领域 manifest 用:`InstanceSummary::loader_version` 存的是给人看的整体
/// 实例 id(见 [`list_instances`]),不能直接喂给 fabric meta(`/loader/<mc>/<ver>/…`),
/// 否则 URL 段错误 → 400。解析不出时返回 `None`(调用方可置空让安装器自动选最新兼容 loader)。
pub fn resolve_loader_version(paths: &GamePaths, id: &str, mc_version: &str) -> Option<String> {
    let chain = crate::version::walk_inherits(id, |cur| {
        let parent = std::fs::read_to_string(paths.version_json(cur))
            .ok()
            .and_then(|raw| VersionHead::parse(&raw))
            .and_then(|h| h.inherits_from);
        Ok::<_, crate::error::CoreError>(crate::version::InheritNode {
            payload: cur.to_string(),
            parent,
        })
    })
    .ok()?;
    for node_id in chain {
        for fam in ["fabric-loader-", "quilt-loader-"] {
            if let Some(rest) = node_id.strip_prefix(fam) {
                // rest = "<loaderver>-<mcver>";剥掉尾部的 "-<mc_version>" 得 loader 版本。
                let lv = rest.strip_suffix(&format!("-{mc_version}")).unwrap_or(rest);
                if !lv.is_empty() {
                    return Some(lv.to_string());
                }
            }
        }
    }
    None
}

/// 扫描一个 game root,列出其中所有**用户实例**。
///
/// 规则:遍历 `versions/` 下每个带可解析 `<id>/<id>.json` 的子目录;但「版本即实例」模型下
/// `versions/` 同时存放真实例与它们 `inheritsFrom` 的核心(原版)/ loader profile —— 后者是被
/// 继承的依赖,不该作为独立实例出现(否则装一个 Fabric 整合包会冒出「原版 + loader + 实例」三行)。
/// 故**凡是被另一目录 `inheritsFrom` 指向的 id 一律隐藏**,只保留继承链顶端的叶子(= 真正可启动
/// 的用户实例)。对每个保留项做轻量读取(只取 id / inheritsFrom)推断 loader 与基础 mc 版本。
/// 任何一步读取或解析失败的目录会被**跳过**(不 panic、不中断其余实例)。
pub fn list_instances(paths: &GamePaths) -> Vec<InstanceSummary> {
    let versions_dir = paths.versions_dir();

    let entries = match std::fs::read_dir(&versions_dir) {
        Ok(e) => e,
        // versions/ 不存在 = 空 root,返回空列表。
        Err(_) => return Vec::new(),
    };

    // 第一遍:收集所有带可解析 <id>/<id>.json 的版本目录(dir, dir_id, head)。
    let mut collected: Vec<(PathBuf, String, VersionHead)> = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        // 目录名即版本 id。
        let dir_id = match dir.file_name().and_then(|n| n.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        // 必须存在 <id>/<id>.json 才算候选(没有版本 json = 纯 natives/临时目录,跳过)。
        let raw = match std::fs::read_to_string(paths.version_json(&dir_id)) {
            Ok(r) => r,
            Err(_) => continue,
        };
        // 轻量解析;解析失败的损坏 json 跳过。
        let head: VersionHead = match serde_json::from_str(&raw) {
            Ok(h) => h,
            Err(_) => continue,
        };
        collected.push((dir, dir_id, head));
    }

    // 被任一目录 inheritsFrom 指向的 id 集合 = 被继承的依赖核心(原版 / loader profile)。
    // 它们不是用户实例,从列表隐藏;只留继承链顶端的叶子。
    let inherited: std::collections::HashSet<String> = collected
        .iter()
        .filter_map(|(_, _, head)| head.inherits_from.clone())
        .collect();

    let mut out: Vec<InstanceSummary> = Vec::new();
    for (dir, dir_id, head) in &collected {
        // 优先用 json 内的 id,回退到目录名。
        let id = head.id.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| dir_id.clone());

        // 被其它实例继承的核心(原版 / loader)= 依赖,非用户实例,隐藏。
        // dir_id 与 id 都查一遍:inheritsFrom 指向的是父目录 id,二者通常相等但容错。
        if inherited.contains(&id) || inherited.contains(dir_id) {
            continue;
        }

        let inherits = head.inherits_from.clone();
        let loader = infer_loader(&id, inherits.as_deref());

        // 基础 mc 版本:沿 inheritsFrom 链解析到根原版(整合包实例可能多层继承)。
        let mc_version = resolve_base_mc_version(paths, &id, inherits.as_deref());

        // 实例名优先取 instance.json 的 name,缺省用 id。
        let config = InstanceConfig::load(&dir.join(INSTANCE_CONFIG_FILE)).unwrap_or_default();
        let name = config.name.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone());

        out.push(InstanceSummary {
            id,
            name,
            mc_version,
            loader,
            // 非原版才暴露 loader_version:用整体 id 作为可读标识(精确解析留给 meta 层)。
            loader_version: match loader {
                LoaderKind::Vanilla => None,
                _ => Some(dir_id.clone()),
            },
            icon: detect_icon(dir), // versions/<id>/icon.png(导入器写入)→ data URL,无则 None。
            last_played: last_played_for(dir),
            running: false, // 运行态由上层进程管理器维护,列表层不感知。
            installed: true,
            realm: config.realm.clone(),
            tags: config.tags.clone(),
        });
    }

    // 第三遍:领域「薄存根」—— 加入领域即写 instance.json(带 realm 绑定)但尚未装核心
    //(无 `<id>/<id>.json`)。这类目录不在 collected 里,这里补成 **pending**(installed=false)
    // 实例,让库里看得到并提供「开始同步」入口;启动被上层拦下,直到 begin 装好核心。
    if let Ok(entries) = std::fs::read_dir(&versions_dir) {
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let dir_id = match dir.file_name().and_then(|n| n.to_str()) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            // 已装核心(有版本 json)的已在上面处理过,跳过。
            if paths.version_json(&dir_id).exists() {
                continue;
            }
            let Ok(config) = InstanceConfig::load(&dir.join(INSTANCE_CONFIG_FILE)) else {
                continue;
            };
            let Some(realm) = config.realm.clone() else { continue };
            out.push(InstanceSummary {
                id: dir_id.clone(),
                name: config.name.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| dir_id.clone()),
                mc_version: realm.mc_version.clone().unwrap_or_default(),
                loader: loader_kind_from_str(realm.loader.as_deref()),
                loader_version: realm.loader_version.clone(),
                icon: detect_icon(&dir),
                last_played: 0,
                running: false,
                installed: false,
                realm: Some(realm),
                tags: config.tags.clone(),
            });
        }
    }

    // 稳定排序:按 id 字典序,保证列表展示顺序确定。
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// 领域薄存根的 loader 字符串 → [`LoaderKind`](mc_types::LoaderKind)(未知 / 缺省视作原版)。
/// 走权威逆函数 [`LoaderKind::from_family`],所以 liteloader / optifine 也能正确归桶
/// (旧的手写 match 漏了它们,会把这些领域误判成原版)。
fn loader_kind_from_str(s: Option<&str>) -> LoaderKind {
    s.and_then(LoaderKind::from_family).unwrap_or(LoaderKind::Vanilla)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn realm_loader_kind_buckets_every_family_not_just_the_old_four() {
        // Regression: the hand-written match dropped liteloader/optifine into the
        // Vanilla bucket. Routing through LoaderKind::from_family fixes that.
        assert_eq!(loader_kind_from_str(Some("liteloader")), LoaderKind::LiteLoader);
        assert_eq!(loader_kind_from_str(Some("optifine")), LoaderKind::OptiFine);
        assert_eq!(loader_kind_from_str(Some("NeoForge")), LoaderKind::NeoForge);
        // Unknown / absent still defaults to Vanilla (a realm stub need not name one).
        assert_eq!(loader_kind_from_str(Some("rift")), LoaderKind::Vanilla);
        assert_eq!(loader_kind_from_str(None), LoaderKind::Vanilla);
    }

    /// 在临时目录里搭一个假的 game root,测试结束自动清理。
    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("mc-core-instance-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        /// 写入 versions/<id>/<id>.json(可选附带 inheritsFrom)。
        fn add_version(&self, id: &str, inherits: Option<&str>) {
            let paths = GamePaths::new(self.path.clone());
            let dir = paths.version_dir(id);
            fs::create_dir_all(&dir).unwrap();
            let json = match inherits {
                Some(p) => format!(r#"{{"id":"{id}","inheritsFrom":"{p}"}}"#),
                None => format!(r#"{{"id":"{id}"}}"#),
            };
            fs::write(paths.version_json(id), json).unwrap();
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn missing_versions_dir_is_empty() {
        let root = TempRoot::new("empty");
        let paths = GamePaths::new(root.path.clone());
        assert!(list_instances(&paths).is_empty());
    }


    #[test]
    fn hides_inherited_cores_lists_leaf_instances() {
        let root = TempRoot::new("mixed");
        // 共享原版核心:被 fabric 与 forge 两个实例 inheritsFrom → 是依赖,应从列表隐藏。
        root.add_version("1.20.1", None);
        root.add_version("fabric-loader-0.15.7-1.20.1", Some("1.20.1"));
        root.add_version("1.20.1-forge-47.2.0", Some("1.20.1"));
        // 没有任何目录继承它的独立原版实例(叶子)→ 应保留。
        root.add_version("1.18.2", None);

        // 一个没有版本 json 的目录(如 natives 残留),应被跳过。
        fs::create_dir_all(root.path.join("versions").join("junk")).unwrap();
        // 一个 json 损坏的目录,应被跳过而不 panic。
        let bad_dir = root.path.join("versions").join("broken");
        fs::create_dir_all(&bad_dir).unwrap();
        fs::write(bad_dir.join("broken.json"), "{ not valid json ").unwrap();

        let paths = GamePaths::new(root.path.clone());
        let list = list_instances(&paths);

        // 隐藏被继承的 1.20.1 核心;保留两个 loader 叶子 + 独立原版叶子 = 3。
        assert_eq!(list.len(), 3, "shared 1.20.1 core hidden; 3 leaf instances remain");

        let by_id = |id: &str| list.iter().find(|s| s.id == id).cloned();

        assert!(
            by_id("1.20.1").is_none(),
            "vanilla core inherited by other instances is hidden, not a phantom instance",
        );

        let standalone = by_id("1.18.2").expect("standalone vanilla leaf is listed");
        assert_eq!(standalone.loader, LoaderKind::Vanilla);
        assert_eq!(standalone.mc_version, "1.18.2");
        assert!(standalone.loader_version.is_none());

        let fabric = by_id("fabric-loader-0.15.7-1.20.1").expect("fabric leaf listed");
        assert_eq!(fabric.loader, LoaderKind::Fabric);
        assert_eq!(fabric.mc_version, "1.20.1");
        assert!(fabric.loader_version.is_some());

        let forge = by_id("1.20.1-forge-47.2.0").expect("forge leaf listed");
        assert_eq!(forge.loader, LoaderKind::Forge);
        assert_eq!(forge.mc_version, "1.20.1");
    }

    #[test]
    fn instance_name_comes_from_config() {
        let root = TempRoot::new("named");
        root.add_version("1.20.1", None);

        let inst = Instance::new("1.20.1", root.path.clone());
        let cfg = InstanceConfig {
            name: Some("Survival World".to_string()),
            ..Default::default()
        };
        inst.save_config(&cfg).unwrap();

        let paths = GamePaths::new(root.path.clone());
        let list = list_instances(&paths);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Survival World");
        assert_eq!(list[0].id, "1.20.1");
    }

    #[test]
    fn instance_config_roundtrip_via_helper() {
        let root = TempRoot::new("cfg");
        let inst = Instance::new("test-id", root.path.clone());

        // 未写盘时返回默认。
        let def = inst.load_config().unwrap();
        assert_eq!(def, InstanceConfig::default());
        assert_eq!(inst.version_id(), "test-id");
        assert!(inst.config_path().ends_with("versions/test-id/instance.json"));

        let cfg = InstanceConfig {
            memory_mb: 6144,
            ..Default::default()
        };
        inst.save_config(&cfg).unwrap();
        assert_eq!(inst.load_config().unwrap().memory_mb, 6144);
    }

    #[test]
    fn base64_encode_matches_known_vectors() {
        // RFC 4648 测试向量,覆盖三种填充情形。
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn detect_icon_reads_png_into_data_url() {
        let root = TempRoot::new("icon");
        root.add_version("1.20.1", None);
        let paths = GamePaths::new(root.path.clone());
        let dir = paths.version_dir("1.20.1");

        // 无 icon.png 时返回 None。
        assert!(detect_icon(&dir).is_none());

        // 写入图标后应被探测并内联为 data URL。
        fs::write(dir.join("icon.png"), b"abc").unwrap();
        assert_eq!(
            detect_icon(&dir).as_deref(),
            Some("data:image/png;base64,YWJj"),
        );

        // 该实例的列表项也应带上同一个 data URL。
        let list = list_instances(&paths);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].icon.as_deref(), Some("data:image/png;base64,YWJj"));
    }

    #[test]
    fn infer_loader_precedence() {
        assert_eq!(infer_loader("neoforge-1.21", None), LoaderKind::NeoForge);
        // neoforge 含 "forge" 子串,但应判定为 NeoForge。
        assert_eq!(infer_loader("1.21-neoforge-21.0.0", Some("1.21")), LoaderKind::NeoForge);
        assert_eq!(infer_loader("1.20.1-forge-47.2.0", Some("1.20.1")), LoaderKind::Forge);
        assert_eq!(infer_loader("fabric-loader-0.15", Some("1.20.1")), LoaderKind::Fabric);
        assert_eq!(infer_loader("quilt-loader-0.20", Some("1.20.1")), LoaderKind::Quilt);
        assert_eq!(infer_loader("1.20.1-OptiFine_HD_U_I6", None), LoaderKind::OptiFine);
        assert_eq!(infer_loader("1.5.2-LiteLoader1.5.2", None), LoaderKind::LiteLoader);
        assert_eq!(infer_loader("1.20.1", None), LoaderKind::Vanilla);
    }
}
