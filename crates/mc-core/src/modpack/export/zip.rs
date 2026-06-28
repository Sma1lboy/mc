//! 导出打包(`ExportToZipTask` 等价):把一组本地文件写进 zip 的 `overrides/<rel>`,**排除**
//! 已 resolved 的键(Prism 的 `setExcludeFiles` 承重点),再把内存里的索引文件注入归档根。
//!
//! 关键不变量(`docs/modules/modpack-export.md` §0 / §8):
//! - **resolved 文件只进索引、不进 overrides**:`exclude` 集里的相对路径在打包时跳过,避免在
//!   包内重复一份(否则包既大又与索引矛盾)。
//! - **失败即清理**:任何一步出错都删掉半成品 `.mrpack/.zip`,绝不留下损坏归档(接收方解压
//!   会失败或得到部分内容)。本模块用 [`PartialOutputGuard`] 在 `Drop` 时兜底删除,只有成功
//!   finish 后才 `disarm`。
//! - **确定性**:文件按传入顺序写(调用方已对 walk 结果排序);索引文件名 / 字节由目标给定。
//!
//! 复用性:空 `overrides_prefix` + 空 `exclude` + 空 `extra_files` = 裸实例备份 zip(无前缀、
//! 不排除、不注入),一个引擎两个调用方(§6)。

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, IoResultExt, Result};
use crate::paths::ensure_dir;

use super::walk::WalkedFile;

/// 打包的全部输入,一次成型(避免长参数列表)。
pub struct ZipPlan<'a> {
    /// overrides 前缀(含尾随 `/`,如 `"overrides/"`);空串 = 直接写到 zip 根(裸备份)。
    pub overrides_prefix: &'a str,
    /// 所有候选文件(相对 game_root + 绝对路径);本函数据 `exclude` 决定是否打包。
    pub files: &'a [WalkedFile],
    /// 已 resolved、**不打进 overrides** 的相对路径键(`setExcludeFiles` 等价)。
    pub exclude: &'a HashSet<String>,
    /// 注入归档根的内存文件(索引 / manifest / modlist),`(归档内路径, 字节)`。
    pub extra_files: &'a [(String, Vec<u8>)],
}

/// 在 `Drop` 时删除一个尚未 `disarm` 的输出文件,保证失败/panic 都不留半成品。
struct PartialOutputGuard {
    path: PathBuf,
    armed: bool,
}

impl PartialOutputGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }
    /// 成功后解除:不再删除。
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PartialOutputGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// 把 `plan` 写成 `dest` 处的 zip 归档,返回写出的路径。
///
/// 流程:建父目录 → 武装清理哨兵 → 写 `extra_files`(索引,放归档根)→ 写未被排除的
/// `overrides/<rel>` 文件 → finish → 解除哨兵。任意错误都触发哨兵删半成品并向上返回。
///
/// zip 路径分隔符统一为 `/`;同名条目按写入顺序去重(extra_files 优先,后续 override 命中
/// 同路径会被跳过——正常情况下 extra_files 在归档根、overrides 在前缀下,不冲突)。
pub fn write_zip(dest: &Path, plan: &ZipPlan<'_>) -> Result<PathBuf> {
    if let Some(parent) = dest.parent() {
        ensure_dir(parent)?;
    }

    let mut guard = PartialOutputGuard::new(dest.to_path_buf());

    let out = std::fs::File::create(dest).with_path(dest)?;
    let mut writer = zip::ZipWriter::new(out);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut written: HashSet<String> = HashSet::new();

    // 1) 注入内存索引文件(归档根)。
    for (name, bytes) in plan.extra_files {
        let name = name.replace('\\', "/");
        if !written.insert(name.clone()) {
            continue;
        }
        writer
            .start_file(name.as_str(), options)
            .map_err(|e| CoreError::Zip(e.to_string()))?;
        writer
            .write_all(bytes)
            .map_err(|e| CoreError::Zip(e.to_string()))?;
    }

    // 2) 写 overrides/<rel>,跳过 resolved(exclude)与已写过的路径。
    for f in plan.files {
        if plan.exclude.contains(&f.rel) {
            continue;
        }
        let entry = if plan.overrides_prefix.is_empty() {
            f.rel.clone()
        } else {
            format!("{}{}", plan.overrides_prefix, f.rel)
        };
        if !written.insert(entry.clone()) {
            continue;
        }
        writer
            .start_file(entry.as_str(), options)
            .map_err(|e| CoreError::Zip(e.to_string()))?;
        let data = std::fs::read(&f.abs).with_path(&f.abs)?;
        writer
            .write_all(&data)
            .map_err(|e| CoreError::io(&f.abs, e))?;
    }

    writer.finish().map_err(|e| CoreError::Zip(e.to_string()))?;
    guard.disarm();
    Ok(dest.to_path_buf())
}

/// 把单个内存文本文件写到 `dest`(`Packaging::SingleTextFile`,如 modlist HTML/MD)。
///
/// 不打 zip、不带 overrides;同样在失败时删半成品(写入中途出错不留残文件)。
pub fn write_text_file(dest: &Path, bytes: &[u8]) -> Result<PathBuf> {
    // write_atomic creates dest's parent dir; no separate ensure_dir.
    let mut guard = PartialOutputGuard::new(dest.to_path_buf());
    crate::fs::write_atomic(dest, bytes)?;
    guard.disarm();
    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Read;

    struct TempRoot {
        path: PathBuf,
    }
    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("mc-core-export-zip-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn walked(root: &Path, rel: &str, bytes: &[u8]) -> WalkedFile {
        let abs = root.join(rel);
        if let Some(p) = abs.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(&abs, bytes).unwrap();
        WalkedFile { rel: rel.to_string(), abs, size: bytes.len() as u64 }
    }

    #[test]
    fn excludes_resolved_keys_and_injects_index() {
        let root = TempRoot::new("exclude");
        let files = vec![
            walked(&root.path, "mods/resolved.jar", b"REMOTE"),
            walked(&root.path, "mods/override.jar", b"LOCAL"),
            walked(&root.path, "config/opts.toml", b"k=1"),
        ];
        let mut exclude = HashSet::new();
        exclude.insert("mods/resolved.jar".to_string());

        let index = (
            "modrinth.index.json".to_string(),
            br#"{"formatVersion":1}"#.to_vec(),
        );
        let dest = root.path.join("out.mrpack");
        let plan = ZipPlan {
            overrides_prefix: "overrides/",
            files: &files,
            exclude: &exclude,
            extra_files: std::slice::from_ref(&index),
        };
        write_zip(&dest, &plan).unwrap();

        let f = fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();

        // 索引注入到根。
        assert!(archive.by_name("modrinth.index.json").is_ok());
        // resolved 文件**不**在 overrides 里(承重点)。
        assert!(
            archive.by_name("overrides/mods/resolved.jar").is_err(),
            "resolved 键必须被排除,不进 overrides"
        );
        // override / config 文件在 overrides 下。
        let mut ov = archive.by_name("overrides/mods/override.jar").unwrap();
        let mut buf = Vec::new();
        ov.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"LOCAL");
        drop(ov);
        assert!(archive.by_name("overrides/config/opts.toml").is_ok());
    }

    #[test]
    fn bare_backup_no_prefix_no_exclude() {
        // 空前缀 + 空排除 + 无注入 = 裸备份:文件原样落 zip 根。
        let root = TempRoot::new("bare");
        let files = vec![walked(&root.path, "mods/a.jar", b"AA")];
        let empty: HashSet<String> = HashSet::new();
        let dest = root.path.join("backup.zip");
        let plan = ZipPlan {
            overrides_prefix: "",
            files: &files,
            exclude: &empty,
            extra_files: &[],
        };
        write_zip(&dest, &plan).unwrap();

        let f = fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        assert!(archive.by_name("mods/a.jar").is_ok(), "裸备份应写到 zip 根");
        assert!(archive.by_name("overrides/mods/a.jar").is_err());
    }

    #[test]
    fn deletes_partial_output_on_read_failure() {
        // 候选文件指向一个不存在的绝对路径 → 读失败 → 半成品应被删除。
        let root = TempRoot::new("partial");
        let bogus = WalkedFile {
            rel: "mods/ghost.jar".into(),
            abs: root.path.join("does-not-exist.jar"),
            size: 10,
        };
        let empty: HashSet<String> = HashSet::new();
        let dest = root.path.join("broken.mrpack");
        let plan = ZipPlan {
            overrides_prefix: "overrides/",
            files: std::slice::from_ref(&bogus),
            exclude: &empty,
            extra_files: &[],
        };
        let err = write_zip(&dest, &plan);
        assert!(err.is_err(), "读不存在文件应失败");
        assert!(!dest.exists(), "失败后半成品归档必须被删除");
    }

    #[test]
    fn write_text_file_roundtrip() {
        let root = TempRoot::new("text");
        let dest = root.path.join("modlist.html");
        write_text_file(&dest, b"<html>hi</html>").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"<html>hi</html>");
    }
}
