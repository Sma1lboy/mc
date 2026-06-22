//! 世界/存档管理 —— 枚举、备份、删除、重命名 `saves/<folder>/` 下的单人世界。
//!
//! 每个世界目录里都有一个 `level.dat`:它是 **gzip 压缩后的 NBT**(命名二进制标签)。
//! 顶层是一个未命名的 root 复合标签,里面有一个 `"Data"` 复合标签承载所有世界元数据
//! (`LevelName` / `GameType` / `LastPlayed` / 种子等)。
//!
//! 设计要点:
//! - 解析尽量"宽容":任何一个世界的 `level.dat` 读不到 / 解坏了,**仍然把目录列出来**
//!   (name 回退为文件夹名,game_mode = "unknown"),绝不因为一个坏存档让整个列表失败。
//!   这与 [`crate::instance::list_instances`] 的"坏目录跳过"语义不同 —— 世界目录存在本身
//!   就是有意义的信息(用户能看到、能删除/备份),所以这里选择"列出但标记未知"。
//! - 种子兼容新老格式:1.16+ 放在 `WorldGenSettings.seed`(Long),更老的版本是
//!   `Data.RandomSeed`(Long)。优先取前者。
//! - 重命名采用"读 → 改 `Data.LevelName` → 重新 gzip 写回"的整树重写,**保留其余所有标签**
//!   (用 `fastnbt::Value` 解析整棵树,只替换一个字段,再 `to_bytes` 序列化),
//!   不会丢失维度数据指针、游戏规则等关键信息。

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use fastnbt::Value;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::error::{CoreError, IoResultExt, Result};
use crate::instance::Instance;

/// `level.dat` 的固定文件名。
const LEVEL_DAT: &str = "level.dat";

/// 单个世界的概要信息(供 UI 列表展示)。
///
/// 字段全部是"已解算好的展示值",不暴露原始 NBT,UI 层无需了解 NBT 结构。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldInfo {
    /// 世界在 `saves/` 下的目录名(稳定标识,用于 backup/delete/rename 的入参)。
    pub folder: String,
    /// 世界显示名(`Data.LevelName`);读不到时回退为 `folder`。
    pub name: String,
    /// 游戏模式:`survival` / `creative` / `adventure` / `spectator` / `unknown`。
    pub game_mode: String,
    /// 上次游玩时间(epoch 毫秒);`0` 表示未知。
    pub last_played: i64,
    /// 世界种子;读不到时为 `None`。
    pub seed: Option<i64>,
    /// 世界目录递归大小(字节),用于展示占用空间。
    pub size_bytes: u64,
}

/// 把 `GameType`(Int)映射为人类可读的游戏模式字符串。
/// 取值约定见 Minecraft wiki:0 生存 / 1 创意 / 2 冒险 / 3 旁观。
fn game_mode_from_int(v: i32) -> &'static str {
    match v {
        0 => "survival",
        1 => "creative",
        2 => "adventure",
        3 => "spectator",
        _ => "unknown",
    }
}

/// 从一个复合标签里取出子标签(若存在且是复合)。
fn get_compound<'a>(map: &'a HashMap<String, Value>, key: &str) -> Option<&'a HashMap<String, Value>> {
    match map.get(key) {
        Some(Value::Compound(c)) => Some(c),
        _ => None,
    }
}

/// 把任意整数型 NBT 标签(Byte/Short/Int/Long)统一读成 i64。
/// `GameType` 名义上是 Int,但不同版本/编辑器可能写成 Byte,这里都兼容。
fn as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Byte(b) => Some(*b as i64),
        Value::Short(s) => Some(*s as i64),
        Value::Int(i) => Some(*i as i64),
        Value::Long(l) => Some(*l),
        _ => None,
    }
}

/// 读取并 gzip 解压 `level.dat`,解析成 `fastnbt::Value`(整棵 root 树)。
/// 失败统一映射为 [`CoreError`],由调用方决定是否降级处理。
fn read_level_dat(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path).with_path(path)?;
    let mut decoder = GzDecoder::new(&bytes[..]);
    let mut buf = Vec::new();
    decoder
        .read_to_end(&mut buf)
        .map_err(|e| CoreError::other(format!("解压 {} 失败: {e}", path.display())))?;
    // root 是未命名复合标签,from_bytes::<Value> 直接给出该复合。
    fastnbt::from_bytes::<Value>(&buf)
        .map_err(|e| CoreError::other(format!("解析 {} 的 NBT 失败: {e}", path.display())))
}

/// 从已解析的 root NBT 中抽取世界元数据,填入 `WorldInfo`(除 folder/size 外的字段)。
/// 任意字段缺失都走默认,不报错。
fn extract_meta(root: &Value, info: &mut WorldInfo) {
    let root_map = match root {
        Value::Compound(c) => c,
        _ => return,
    };
    // 所有世界元数据都在 "Data" 复合标签下。
    let data = match get_compound(root_map, "Data") {
        Some(d) => d,
        None => return,
    };

    if let Some(Value::String(name)) = data.get("LevelName") {
        if !name.is_empty() {
            info.name = name.clone();
        }
    }

    if let Some(gt) = data.get("GameType").and_then(as_i64) {
        info.game_mode = game_mode_from_int(gt as i32).to_string();
    }

    if let Some(lp) = data.get("LastPlayed").and_then(as_i64) {
        info.last_played = lp;
    }

    // 种子:优先 1.16+ 的 WorldGenSettings.seed,回退老版本的 RandomSeed。
    let seed = get_compound(data, "WorldGenSettings")
        .and_then(|wgs| wgs.get("seed"))
        .and_then(as_i64)
        .or_else(|| data.get("RandomSeed").and_then(as_i64));
    if let Some(s) = seed {
        info.seed = Some(s);
    }
}

/// 递归统计目录占用的字节数。无法读取的子项按 0 计,不中断统计。
fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => total += dir_size(&p),
            Ok(ft) if ft.is_file() => {
                if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            }
            // 符号链接等其它类型不计入,避免重复/越界统计。
            _ => {}
        }
    }
    total
}

/// 列出某实例下的所有世界。
///
/// 扫描 `inst.saves_dir()` 下每个**包含 `level.dat`** 的子目录,逐个解析元数据。
/// 解析失败(读不到/解坏)的世界仍然列出,只是字段走默认值(name=folder,
/// game_mode="unknown"),保证"目录存在 = 可见"。返回结果按 folder 字典序稳定排序。
pub fn list_worlds(inst: &Instance) -> Vec<WorldInfo> {
    let saves_dir = inst.saves_dir();
    let entries = match std::fs::read_dir(&saves_dir) {
        Ok(e) => e,
        // saves/ 不存在 = 没有世界。
        Err(_) => return Vec::new(),
    };

    let mut out: Vec<WorldInfo> = Vec::new();

    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        // 没有 level.dat 的目录不是一个世界(可能是用户随手建的空文件夹),跳过。
        let level_path = dir.join(LEVEL_DAT);
        if !level_path.is_file() {
            continue;
        }

        let folder = match dir.file_name().and_then(|n| n.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };

        // 先填默认值:即使后续解析失败,也至少能呈现一个可操作的世界条目。
        let mut info = WorldInfo {
            folder: folder.clone(),
            name: folder.clone(),
            game_mode: "unknown".to_string(),
            last_played: 0,
            seed: None,
            size_bytes: dir_size(&dir),
        };

        // 解析成功才覆盖默认值;失败则保留上面的降级展示。
        if let Ok(root) = read_level_dat(&level_path) {
            extract_meta(&root, &mut info);
        }

        out.push(info);
    }

    out.sort_by(|a, b| a.folder.cmp(&b.folder));
    out
}

/// 定位某个世界目录,校验其确实存在(否则报 [`CoreError::InstanceNotFound`])。
fn world_dir(inst: &Instance, folder: &str) -> Result<PathBuf> {
    let dir = inst.saves_dir().join(folder);
    if !dir.is_dir() {
        return Err(CoreError::InstanceNotFound(format!("world `{folder}`")));
    }
    Ok(dir)
}

/// 把世界目录递归写入一个已打开的 [`ZipWriter`]。
/// `prefix` 是 zip 内部的相对前缀(用世界文件夹名作为根,保留目录结构)。
fn zip_dir_recursive<W: std::io::Write + std::io::Seek>(
    writer: &mut ZipWriter<W>,
    base: &Path,
    current: &Path,
    options: SimpleFileOptions,
) -> Result<()> {
    use std::io::Write;

    for entry in std::fs::read_dir(current).with_path(current)? {
        let entry = entry.with_path(current)?;
        let path = entry.path();
        // zip 内部用相对于 base 父目录的路径,保证根目录为世界文件夹名本身。
        let rel = path
            .strip_prefix(base.parent().unwrap_or(base))
            .unwrap_or(&path);
        // zip 规范统一用 '/' 作分隔符,避免 Windows 反斜杠污染归档。
        let name = rel.to_string_lossy().replace('\\', "/");

        let ft = entry.file_type().with_path(&path)?;
        if ft.is_dir() {
            // 显式写入目录条目(以 '/' 结尾),保留空目录结构。
            writer
                .add_directory(format!("{name}/"), options)
                .map_err(|e| CoreError::Zip(e.to_string()))?;
            zip_dir_recursive(writer, base, &path, options)?;
        } else if ft.is_file() {
            writer
                .start_file(name, options)
                .map_err(|e| CoreError::Zip(e.to_string()))?;
            let data = std::fs::read(&path).with_path(&path)?;
            writer
                .write_all(&data)
                .map_err(|e| CoreError::io(&path, e))?;
        }
        // 其它类型(符号链接等)不打包。
    }
    Ok(())
}

/// 把世界目录打成 zip 备份到 `dest_dir/<folder>-backup.zip`,返回该 zip 路径。
///
/// 归档内部以世界文件夹名为根目录(便于解压后直接是一个可用的 `saves/<folder>/`)。
/// 自动创建 `dest_dir`。使用 Deflated 压缩(存档主要是文本/小文件,压缩收益明显)。
pub fn backup_world(inst: &Instance, folder: &str, dest_dir: &Path) -> Result<PathBuf> {
    let src = world_dir(inst, folder)?;

    std::fs::create_dir_all(dest_dir).with_path(dest_dir)?;
    let zip_path = dest_dir.join(format!("{folder}-backup.zip"));

    let file = std::fs::File::create(&zip_path).with_path(&zip_path)?;
    let mut writer = ZipWriter::new(file);
    let options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip_dir_recursive(&mut writer, &src, &src, options)?;

    writer.finish().map_err(|e| CoreError::Zip(e.to_string()))?;
    Ok(zip_path)
}

/// 删除整个世界目录。
///
/// 优先移入系统回收站([`trash::delete`],可恢复、对用户更安全);回收站不可用时
/// (如某些 headless / 容器环境)回退到 [`std::fs::remove_dir_all`] 永久删除。
pub fn delete_world(inst: &Instance, folder: &str) -> Result<()> {
    let dir = world_dir(inst, folder)?;

    if trash::delete(&dir).is_ok() {
        return Ok(());
    }
    // 回收站不可用:回退到永久删除。
    std::fs::remove_dir_all(&dir).with_path(&dir)
}

/// 重命名世界的**显示名**(`Data.LevelName`),不改变其文件夹名。
///
/// 流程:读 `level.dat` → gzip 解压 → 解析为整棵 `Value` 树 → 改写 `Data.LevelName`
/// → `to_bytes` 重新序列化 → gzip 压缩 → 原子写回。整树重写保留其余所有标签
/// (维度指针、游戏规则、玩家数据等),不会丢失任何信息。
///
/// 写回用 [`crate::fs::write_atomic`]:崩溃也不会留下半截损坏的 `level.dat`。
pub fn rename_world(inst: &Instance, folder: &str, new_name: &str) -> Result<()> {
    let dir = world_dir(inst, folder)?;
    let level_path = dir.join(LEVEL_DAT);

    // 解析整棵树(这里要求 level.dat 可读可解 —— 重命名一个坏存档没有意义)。
    let mut root = read_level_dat(&level_path)?;

    // 定位 root → Data 复合,改写 LevelName。
    let root_map = match &mut root {
        Value::Compound(c) => c,
        _ => return Err(CoreError::other("level.dat 根标签不是复合类型,无法重命名")),
    };
    let data = match root_map.get_mut("Data") {
        Some(Value::Compound(d)) => d,
        _ => return Err(CoreError::other("level.dat 缺少 Data 复合标签,无法重命名")),
    };
    data.insert("LevelName".to_string(), Value::String(new_name.to_string()));

    // 重新序列化为 NBT(默认 SerOpts 以空串作 root 名,与 Minecraft 的 level.dat 一致)。
    let nbt_bytes = fastnbt::to_bytes(&root)
        .map_err(|e| CoreError::other(format!("序列化 NBT 失败: {e}")))?;

    // gzip 压缩后原子写回。
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        use std::io::Write;
        encoder
            .write_all(&nbt_bytes)
            .map_err(|e| CoreError::io(&level_path, e))?;
    }
    let gzipped = encoder
        .finish()
        .map_err(|e| CoreError::io(&level_path, e))?;

    crate::fs::write_atomic(&level_path, &gzipped)
}

/// 从一个 `.zip` 导入一个世界到实例 `saves/` 下,返回新世界的文件夹名。
///
/// 兼容两种常见布局:`level.dat` 在 zip 根(根即世界),或位于单层子目录下
/// (`<World>/level.dat`,如 [`backup_world`] 导出的备份)。取**最浅**的 `level.dat`
/// 所在目录作为归档根,把该子树解压到 `saves/<folder>/`(经 zip-slip 收口)。
///
/// 文件夹名取 zip 文件名(去 `.zip` 与 `-backup` 后缀)、清洗成文件系统安全名,并在
/// `saves/` 下唯一化避免覆盖已有世界。zip 内不含 `level.dat` 时报错(不是一个世界存档)。
pub fn import_world_zip(inst: &Instance, source: &Path) -> Result<String> {
    use crate::modpack::import::archive::ZipArchiveIndex;
    use crate::modpack::import::ArchiveIndex;

    let mut idx = ZipArchiveIndex::open(source)?;

    // 在条目里找 level.dat:根级 "level.dat",或某目录下 ".../level.dat"。取最浅的一个。
    let archive_root = idx
        .entries()
        .iter()
        .filter_map(|e| {
            if e == LEVEL_DAT {
                Some(String::new())
            } else {
                e.strip_suffix(&format!("/{LEVEL_DAT}")).map(|p| p.to_string())
            }
        })
        .min_by_key(|root| root.matches('/').count())
        .ok_or_else(|| CoreError::other("zip 内没有 level.dat,不是一个世界存档"))?;

    // 目标文件夹名:zip 文件名去扩展名与 -backup 后缀 → 清洗 → 唯一化。
    let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("World");
    let base = sanitize_world_folder(stem.strip_suffix("-backup").unwrap_or(stem));
    let saves = inst.saves_dir();
    let folder = unique_world_folder(&saves, &base);
    let dest = saves.join(&folder);

    // 解压子树到目标目录;失败则尽力清掉半成品目录,避免留下不可用的残骸。
    if let Err(e) = idx.extract_subtree(&archive_root, &dest) {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }

    // 兜底校验:确实落出了 level.dat,否则视为导入失败。
    if !dest.join(LEVEL_DAT).is_file() {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(CoreError::other("解压后未找到 level.dat,导入失败"));
    }

    Ok(folder)
}

/// 把任意字符串清洗成文件系统安全的世界文件夹名:路径分隔符/保留字/控制符与空白归一为
/// `-`,去首尾 `-`;空结果回退 `World`。保留 unicode(中文世界名可直接作目录名)。
fn sanitize_world_folder(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.trim().chars() {
        let bad = ch.is_whitespace()
            || ch.is_control()
            || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
        if bad {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        } else {
            out.push(ch);
            prev_dash = false;
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() { "World".to_string() } else { s }
}

/// 在 `saves` 下为 `base` 找一个不冲突的文件夹名(冲突则追加 `-2`/`-3`…)。
fn unique_world_folder(saves: &Path, base: &str) -> String {
    if !saves.join(base).exists() {
        return base.to_string();
    }
    (2u32..)
        .map(|n| format!("{base}-{n}"))
        .find(|cand| !saves.join(cand).exists())
        .unwrap_or_else(|| base.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 临时 game root,Drop 时自动清理。
    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "mc-core-world-test-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn instance(&self) -> Instance {
            // version_id 不影响 saves 路径解析,这里给个占位 id。
            Instance::new("test-version", self.path.clone())
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// 构造一棵最小 level.dat 的 NBT 树:root → Data{LevelName, GameType, LastPlayed, RandomSeed}。
    fn build_level_value(name: &str, game_type: i32, last_played: i64, seed: i64) -> Value {
        let mut data = HashMap::new();
        data.insert("LevelName".to_string(), Value::String(name.to_string()));
        data.insert("GameType".to_string(), Value::Int(game_type));
        data.insert("LastPlayed".to_string(), Value::Long(last_played));
        data.insert("RandomSeed".to_string(), Value::Long(seed));

        let mut root = HashMap::new();
        root.insert("Data".to_string(), Value::Compound(data));
        Value::Compound(root)
    }

    /// 把一棵 NBT 树序列化 + gzip 写到 saves/<world>/level.dat。
    fn write_world(inst: &Instance, world: &str, value: &Value) {
        let dir = inst.saves_dir().join(world);
        std::fs::create_dir_all(&dir).unwrap();
        let nbt = fastnbt::to_bytes(value).unwrap();
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&nbt).unwrap();
        let gz = enc.finish().unwrap();
        std::fs::write(dir.join(LEVEL_DAT), gz).unwrap();
    }

    #[test]
    fn lists_world_with_parsed_name_and_mode() {
        let root = TempRoot::new("list");
        let inst = root.instance();

        let value = build_level_value("My Creative World", 1, 1_700_000_000_000, 42);
        write_world(&inst, "world1", &value);

        let worlds = list_worlds(&inst);
        assert_eq!(worlds.len(), 1);
        let w = &worlds[0];
        assert_eq!(w.folder, "world1");
        assert_eq!(w.name, "My Creative World");
        assert_eq!(w.game_mode, "creative");
        assert_eq!(w.last_played, 1_700_000_000_000);
        assert_eq!(w.seed, Some(42));
        assert!(w.size_bytes > 0, "size should count level.dat bytes");
    }

    #[test]
    fn import_world_zip_roundtrips_from_backup() {
        let root = TempRoot::new("import");
        let inst = root.instance();
        write_world(&inst, "world1", &build_level_value("Round Trip", 0, 123, 9));

        // 备份得到 world1-backup.zip(zip 内根目录为 world1/)。
        let backup_dir = root.path.join("backups");
        let zip = backup_world(&inst, "world1", &backup_dir).unwrap();
        assert!(zip.is_file());

        // 导入:文件名去 -backup → world1,已存在 → 唯一化为 world1-2。
        let folder = import_world_zip(&inst, &zip).unwrap();
        assert_eq!(folder, "world1-2");
        assert!(inst.saves_dir().join("world1-2").join(LEVEL_DAT).is_file());

        // 现在有两个世界,导入的那个元数据可正常解析。
        let worlds = list_worlds(&inst);
        assert_eq!(worlds.len(), 2);
        let imported = worlds.iter().find(|w| w.folder == "world1-2").unwrap();
        assert_eq!(imported.name, "Round Trip");
    }

    #[test]
    fn import_world_zip_rejects_non_world() {
        let root = TempRoot::new("import-bad");
        let inst = root.instance();
        // 造一个不含 level.dat 的 zip。
        let zip_path = root.path.join("notworld.zip");
        let f = std::fs::File::create(&zip_path).unwrap();
        let mut zw = ZipWriter::new(f);
        zw.start_file("readme.txt", SimpleFileOptions::default()).unwrap();
        zw.write_all(b"hi").unwrap();
        zw.finish().unwrap();

        assert!(import_world_zip(&inst, &zip_path).is_err());
        // 失败不应留下任何世界目录。
        assert!(list_worlds(&inst).is_empty());
    }

    #[test]
    fn survival_mode_mapping() {
        let root = TempRoot::new("survival");
        let inst = root.instance();
        write_world(&inst, "s", &build_level_value("S", 0, 0, 7));
        let worlds = list_worlds(&inst);
        assert_eq!(worlds[0].game_mode, "survival");
    }

    #[test]
    fn corrupt_level_dat_still_listed_as_unknown() {
        let root = TempRoot::new("corrupt");
        let inst = root.instance();
        // 写一个非 gzip / 非 NBT 的 level.dat:解析必然失败,但世界应仍被列出。
        let dir = inst.saves_dir().join("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LEVEL_DAT), b"not a valid level.dat").unwrap();

        let worlds = list_worlds(&inst);
        assert_eq!(worlds.len(), 1);
        assert_eq!(worlds[0].folder, "broken");
        assert_eq!(worlds[0].name, "broken", "name falls back to folder");
        assert_eq!(worlds[0].game_mode, "unknown");
        assert_eq!(worlds[0].seed, None);
    }

    #[test]
    fn directory_without_level_dat_is_skipped() {
        let root = TempRoot::new("skip");
        let inst = root.instance();
        // 空目录(无 level.dat)不算世界。
        std::fs::create_dir_all(inst.saves_dir().join("not-a-world")).unwrap();
        write_world(&inst, "real", &build_level_value("Real", 0, 0, 1));

        let worlds = list_worlds(&inst);
        assert_eq!(worlds.len(), 1);
        assert_eq!(worlds[0].folder, "real");
    }

    #[test]
    fn missing_saves_dir_returns_empty() {
        let root = TempRoot::new("nosaves");
        let inst = root.instance();
        assert!(list_worlds(&inst).is_empty());
    }

    #[test]
    fn worldgensettings_seed_preferred_over_random_seed() {
        let root = TempRoot::new("seed");
        let inst = root.instance();

        // 同时存在 WorldGenSettings.seed 与 RandomSeed,应取前者(1.16+ 格式)。
        let mut data = HashMap::new();
        data.insert("LevelName".to_string(), Value::String("Seeded".to_string()));
        data.insert("GameType".to_string(), Value::Int(0));
        data.insert("RandomSeed".to_string(), Value::Long(111));
        let mut wgs = HashMap::new();
        wgs.insert("seed".to_string(), Value::Long(999));
        data.insert("WorldGenSettings".to_string(), Value::Compound(wgs));
        let mut rootmap = HashMap::new();
        rootmap.insert("Data".to_string(), Value::Compound(data));

        write_world(&inst, "w", &Value::Compound(rootmap));

        let worlds = list_worlds(&inst);
        assert_eq!(worlds[0].seed, Some(999));
    }

    #[test]
    fn rename_changes_name_preserves_other_tags() {
        let root = TempRoot::new("rename");
        let inst = root.instance();

        // 原始世界带种子/模式/时间,重命名后这些都应保留。
        write_world(&inst, "w", &build_level_value("Old Name", 2, 12345, 678));

        rename_world(&inst, "w", "New Name").unwrap();

        let worlds = list_worlds(&inst);
        assert_eq!(worlds.len(), 1);
        let w = &worlds[0];
        assert_eq!(w.name, "New Name");
        // 其它标签必须无损保留。
        assert_eq!(w.game_mode, "adventure");
        assert_eq!(w.last_played, 12345);
        assert_eq!(w.seed, Some(678));
    }

    #[test]
    fn backup_creates_zip_with_world_contents() {
        let root = TempRoot::new("backup");
        let inst = root.instance();

        write_world(&inst, "w", &build_level_value("W", 0, 0, 1));
        // 加一个嵌套文件,验证递归打包。
        let region = inst.saves_dir().join("w").join("region");
        std::fs::create_dir_all(&region).unwrap();
        std::fs::write(region.join("r.0.0.mca"), b"chunk-bytes").unwrap();

        let dest = root.path.join("backups");
        let zip_path = backup_world(&inst, "w", &dest).unwrap();

        assert!(zip_path.exists());
        assert_eq!(zip_path.file_name().unwrap(), "w-backup.zip");

        // 读回 zip,确认包含世界根目录下的 level.dat 与嵌套 region 文件。
        let f = std::fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();
        assert!(names.iter().any(|n| n == "w/level.dat"), "names: {names:?}");
        assert!(
            names.iter().any(|n| n == "w/region/r.0.0.mca"),
            "names: {names:?}"
        );
    }

    #[test]
    fn delete_removes_world_directory() {
        let root = TempRoot::new("delete");
        let inst = root.instance();
        write_world(&inst, "w", &build_level_value("W", 0, 0, 1));
        let dir = inst.saves_dir().join("w");
        assert!(dir.exists());

        delete_world(&inst, "w").unwrap();
        assert!(!dir.exists(), "world dir should be gone after delete");
    }

    #[test]
    fn operations_on_missing_world_error() {
        let root = TempRoot::new("missing");
        let inst = root.instance();
        assert!(matches!(
            backup_world(&inst, "nope", &root.path),
            Err(CoreError::InstanceNotFound(_))
        ));
        assert!(matches!(
            delete_world(&inst, "nope"),
            Err(CoreError::InstanceNotFound(_))
        ));
        assert!(matches!(
            rename_world(&inst, "nope", "x"),
            Err(CoreError::InstanceNotFound(_))
        ));
    }
}
