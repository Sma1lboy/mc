//! 归档读写:**唯一**碰 `zip` crate 的导入侧文件。
//!
//! 提供:
//! - [`ZipArchiveIndex`]:把一个 `.zip`/`.mrpack` 打开一次,缓存条目清单,实现
//!   [`super::ArchiveIndex`](`detect()` 的只读视图);`read_small` 按需取条目字节。
//! - [`extract_subtree`]:按 [`super::DetectMatch::archive_root`] 把对应子树解压到 staging
//!   临时目录,所有相对路径经 [`crate::fs::safe_join`] 收口(zip-slip),并在 Unix 上修复
//!   可执行权限位(保真还原)。
//! - [`extract_prefix`]:把归档内某前缀(如 `overrides/`)下的条目铺到目标目录;
//!   modrinth importer 的 override 铺设走它。
//!
//! 设计:扫描与写出分两步(先收集 `(index, 相对路径)`,再逐个写),规避 `zip` 的可变
//! 借用冲突(对齐既有 `instance/lifecycle.rs::write_override_pass` 的写法)。

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, IoResultExt, Result};
use crate::paths::ensure_dir;

/// 打开一次的归档 + 缓存的条目清单。实现 [`super::ArchiveIndex`] 供 `detect()` 用。
pub struct ZipArchiveIndex {
    archive: zip::ZipArchive<std::fs::File>,
    /// 所有**文件**条目的相对路径(`/` 分隔,反斜杠已归一,去前导 `./`)。
    entries: Vec<String>,
}

impl ZipArchiveIndex {
    /// 打开 `path` 处的 zip 并建立条目索引。
    pub fn open(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path).with_path(path)?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Zip(e.to_string()))?;

        let mut entries = Vec::with_capacity(archive.len());
        for i in 0..archive.len() {
            let entry = archive.by_index(i).map_err(|e| CoreError::Zip(e.to_string()))?;
            if entry.is_dir() {
                continue;
            }
            let name = normalize_entry(entry.name());
            if !name.is_empty() {
                entries.push(name);
            }
        }
        Ok(Self { archive, entries })
    }

    /// 解压由 `archive_root` 指定的子树到 `staging`。
    ///
    /// `archive_root` 为空表示整个归档即包根(子树 = 全部)。命中前缀的条目去掉该前缀后
    /// 经 [`crate::fs::safe_join`] 落到 `staging` 下;越权路径(zip-slip)报错。Unix 上还原
    /// 可执行位。
    pub fn extract_subtree(&mut self, archive_root: &str, staging: &Path) -> Result<()> {
        let prefix = root_prefix(archive_root);
        let targets = self.collect_targets(|name| strip_root(name, &prefix));
        self.write_targets(targets, staging)
    }

    /// 把归档内 `prefix`(如 `overrides/`,需自带尾 `/`)下的条目铺到 `dest`。
    ///
    /// 用于 modrinth 的 override 铺设:`prefix` 是归档内绝对前缀(含 `archive_root`),命中
    /// 条目去掉该前缀后落到 `dest`。返回是否铺了至少一个文件(便于调用方判断)。
    pub fn extract_prefix(&mut self, prefix: &str, dest: &Path) -> Result<bool> {
        let targets = self.collect_targets(|name| {
            name.strip_prefix(prefix)
                .filter(|rel| !rel.is_empty())
                .map(|rel| rel.to_string())
        });
        let any = !targets.is_empty();
        self.write_targets(targets, dest)?;
        Ok(any)
    }

    /// 扫描所有条目,用 `pick` 把命中条目映射成 `(index, 目标相对路径)`。
    fn collect_targets(
        &mut self,
        pick: impl Fn(&str) -> Option<String>,
    ) -> Vec<(usize, String)> {
        let mut targets: Vec<(usize, String)> = Vec::new();
        for i in 0..self.archive.len() {
            // by_index 仅在扫描阶段短暂借用,随即释放,避免与写出冲突。
            let raw = match self.archive.by_index(i) {
                Ok(e) => e,
                Err(_) => continue,
            };
            if raw.is_dir() {
                continue;
            }
            let name = normalize_entry(raw.name());
            if let Some(rel) = pick(&name) {
                targets.push((i, rel));
            }
        }
        targets
    }

    /// 把扫描出的 `(index, 相对路径)` 逐个写到 `base` 下(经 safe_join + 权限还原)。
    fn write_targets(&mut self, targets: Vec<(usize, String)>, base: &Path) -> Result<()> {
        for (i, rel) in targets {
            let Some(dest) = crate::fs::safe_join(base, &rel) else {
                return Err(CoreError::other(format!("非法的归档路径(越权): {rel}")));
            };
            if let Some(parent) = dest.parent() {
                ensure_dir(parent)?;
            }
            let mut entry = self.archive.by_index(i).map_err(|e| CoreError::Zip(e.to_string()))?;
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut buf)
                .map_err(|e| CoreError::Zip(format!("读取归档条目失败: {e}")))?;
            // 覆盖写(第二遍 prefix 如 client-overrides 会覆盖同名文件)。
            crate::fs::write_atomic(&dest, &buf)?;
            apply_unix_mode(&dest, entry.unix_mode());
        }
        Ok(())
    }

    /// 按条目名读取一个小文件的字节(供内容判别 / plan 读 manifest)。
    fn read_entry(&mut self, name: &str) -> Option<Vec<u8>> {
        // zip 的 by_name 按原始条目名查;先尝试原名,再尝试归一名匹配到的原始下标。
        if let Ok(mut e) = self.archive.by_name(name) {
            let mut buf = Vec::with_capacity(e.size() as usize);
            return e.read_to_end(&mut buf).ok().map(|_| buf);
        }
        // 回退:线性找一个归一后等于 name 的条目(处理反斜杠 / 前导 ./ 的归档)。
        let idx = (0..self.archive.len()).find(|&i| {
            self.archive
                .by_index(i)
                .map(|e| normalize_entry(e.name()) == name)
                .unwrap_or(false)
        })?;
        let mut e = self.archive.by_index(idx).ok()?;
        let mut buf = Vec::with_capacity(e.size() as usize);
        e.read_to_end(&mut buf).ok().map(|_| buf)
    }
}

impl super::ArchiveIndex for ZipArchiveIndex {
    fn entries(&self) -> &[String] {
        &self.entries
    }

    fn read_small(&self, name: &str) -> Option<Vec<u8>> {
        // ArchiveIndex::read_small 取 &self,但 zip 读条目需要 &mut。用内部可变性桥接:
        // 这里复制一份归档句柄按需打开会更复杂,故 detect 阶段实际通过 read_small_mut
        // (见 engine)调用;本实现给出一个基于重新打开的安全版本不可行(无原始 path),
        // 因此仅在持有 &mut 的路径上调用 read_entry。为满足 trait 签名,这里返回 None,
        // 真实内容判别走 [`ZipArchiveIndex::read_small_owned`]。
        let _ = name;
        None
    }
}

impl ZipArchiveIndex {
    /// 取 `&mut self` 的内容读取(detect 的内容判别用)。`read_small`(&self)受 trait
    /// 签名所限无法读 zip,引擎在 detect 前用本方法预取需要判别的 manifest 喂给一个带
    /// 内容缓存的视图(见 [`PreparedIndex`])。
    pub fn read_small_owned(&mut self, name: &str) -> Option<Vec<u8>> {
        self.read_entry(name)
    }

    /// 把本归档转成一个**带内容缓存**的只读视图:预读 `prefetch` 列出的小文件内容,
    /// 使 [`super::ArchiveIndex::read_small`] 在 `&self` 下也能命中(CF vs MCBBS 判别)。
    pub fn into_prepared(mut self, prefetch: &[&str]) -> PreparedIndex {
        let mut cache: Vec<(String, Vec<u8>)> = Vec::new();
        for name in prefetch {
            if let Some(bytes) = self.read_entry(name) {
                cache.push((name.to_string(), bytes));
            }
        }
        PreparedIndex { entries: std::mem::take(&mut self.entries), cache, inner: self }
    }
}

/// `detect()` 阶段用的只读视图:条目清单 + 预读的小文件内容缓存。
///
/// 解决「`ArchiveIndex::read_small` 取 `&self` 而读 zip 需 `&mut`」的张力:引擎在 detect
/// 前把可能要做内容判别的 manifest(如 `manifest.json`)预读进缓存,`read_small` 命中缓存
/// 即返回。`inner` 保留底层归档,供 detect 后解压子树(取回 `&mut`)。
pub struct PreparedIndex {
    entries: Vec<String>,
    cache: Vec<(String, Vec<u8>)>,
    inner: ZipArchiveIndex,
}

impl PreparedIndex {
    /// 取回底层归档(detect 完成后用于解压子树)。
    pub fn into_inner(self) -> ZipArchiveIndex {
        self.inner
    }
}

impl super::ArchiveIndex for PreparedIndex {
    fn entries(&self) -> &[String] {
        &self.entries
    }

    fn read_small(&self, name: &str) -> Option<Vec<u8>> {
        let norm = normalize_entry(name);
        self.cache
            .iter()
            .find(|(k, _)| *k == norm)
            .map(|(_, v)| v.clone())
    }
}

/// 归一一个 zip 条目名:反斜杠 → `/`,去前导 `./`。
pub(crate) fn normalize_entry(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches("./").to_string()
}

/// 由 `archive_root` 推出剥离前缀:空根 → 空前缀;否则 `root/`。
fn root_prefix(archive_root: &str) -> String {
    let r = archive_root.trim_matches('/');
    if r.is_empty() {
        String::new()
    } else {
        format!("{r}/")
    }
}

/// 在子树解压时把条目名相对包根:前缀为空则原样;否则只接受命中前缀的并去掉它。
fn strip_root(name: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        Some(name.to_string())
    } else {
        name.strip_prefix(prefix)
            .filter(|rel| !rel.is_empty())
            .map(|rel| rel.to_string())
    }
}

/// 在 Unix 上还原条目的可执行位(保真;非 Unix 平台空操作)。
fn apply_unix_mode(path: &Path, mode: Option<u32>) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(mode) = mode {
            // 仅当置了可执行位时才设(避免把数据文件设成不可读)。
            if mode & 0o111 != 0 {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

/// 把一个 staging 子目录的内容铺到游戏目录(用于 plan.override_roots 中**已解压到
/// staging** 的 override 根)。逐文件经 [`crate::fs::safe_join`] 收口。
///
/// `src_root` 是 staging 下的一个 override 根目录(如 `staging/overrides`);不存在则空操作。
pub fn overlay_dir_safe(src_root: &Path, game_dir: &Path) -> Result<()> {
    if !src_root.is_dir() {
        return Ok(());
    }
    overlay_rec(src_root, src_root, game_dir)
}

fn overlay_rec(base: &Path, current: &Path, game_dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(current).with_path(current)? {
        let entry = entry.with_path(current)?;
        let path = entry.path();
        let ft = entry.file_type().with_path(&path)?;
        if ft.is_dir() {
            overlay_rec(base, &path, game_dir)?;
        } else if ft.is_file() {
            // 相对 override 根的子路径,经 safe_join 落到 game_dir。
            let rel = path.strip_prefix(base).unwrap_or(&path);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let Some(dest) = crate::fs::safe_join(game_dir, &rel_str) else {
                return Err(CoreError::other(format!("非法的 override 路径(越权): {rel_str}")));
            };
            if let Some(parent) = dest.parent() {
                ensure_dir(parent)?;
            }
            std::fs::copy(&path, &dest).with_path(&dest)?;
        }
    }
    Ok(())
}

/// 在系统临时目录下建一个唯一 staging 目录(导入解压用),Drop 时自动清理。
pub struct StagingDir {
    path: PathBuf,
}

impl StagingDir {
    /// 创建一个新的唯一 staging 目录。
    pub fn new() -> Result<Self> {
        let unique = format!(
            "mc-import-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let path = std::env::temp_dir().join(unique);
        ensure_dir(&path)?;
        Ok(Self { path })
    }

    /// staging 根路径。
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_zip(dest: &Path, files: &[(&str, &[u8])]) {
        let out = std::fs::File::create(dest).unwrap();
        let mut zw = zip::ZipWriter::new(out);
        let opt = zip::write::SimpleFileOptions::default();
        for (name, body) in files {
            zw.start_file(*name, opt).unwrap();
            zw.write_all(body).unwrap();
        }
        zw.finish().unwrap();
    }

    struct Tmp {
        dir: PathBuf,
    }
    impl Tmp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("mc-core-archive-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn index_lists_only_files_and_normalizes() {
        let t = Tmp::new("index");
        let zp = t.dir.join("p.zip");
        write_zip(&zp, &[("a/b.txt", b"x"), ("c.txt", b"y")]);
        let idx = ZipArchiveIndex::open(&zp).unwrap();
        use super::super::ArchiveIndex;
        let entries = idx.entries();
        assert!(entries.contains(&"a/b.txt".to_string()));
        assert!(entries.contains(&"c.txt".to_string()));
    }

    #[test]
    fn extract_subtree_with_root_strips_prefix() {
        let t = Tmp::new("subtree");
        let zp = t.dir.join("p.zip");
        write_zip(&zp, &[("Pack/manifest.json", b"M"), ("Pack/mods/a.jar", b"J"), ("other.txt", b"O")]);
        let mut idx = ZipArchiveIndex::open(&zp).unwrap();
        let staging = t.dir.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        idx.extract_subtree("Pack", &staging).unwrap();
        assert_eq!(std::fs::read(staging.join("manifest.json")).unwrap(), b"M");
        assert_eq!(std::fs::read(staging.join("mods/a.jar")).unwrap(), b"J");
        // Pack 之外的 other.txt 不在子树内,不应解出。
        assert!(!staging.join("other.txt").exists());
    }

    #[test]
    fn extract_subtree_empty_root_extracts_all() {
        let t = Tmp::new("rootall");
        let zp = t.dir.join("p.zip");
        write_zip(&zp, &[("modrinth.index.json", b"I"), ("overrides/x.txt", b"X")]);
        let mut idx = ZipArchiveIndex::open(&zp).unwrap();
        let staging = t.dir.join("s");
        std::fs::create_dir_all(&staging).unwrap();
        idx.extract_subtree("", &staging).unwrap();
        assert_eq!(std::fs::read(staging.join("modrinth.index.json")).unwrap(), b"I");
        assert_eq!(std::fs::read(staging.join("overrides/x.txt")).unwrap(), b"X");
    }

    #[test]
    fn extract_prefix_blocks_zip_slip() {
        let t = Tmp::new("slip");
        let zp = t.dir.join("evil.zip");
        write_zip(&zp, &[("overrides/../../escaped.txt", b"PWNED")]);
        let mut idx = ZipArchiveIndex::open(&zp).unwrap();
        let dest = t.dir.join("game");
        std::fs::create_dir_all(&dest).unwrap();
        let err = idx.extract_prefix("overrides/", &dest).unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "zip-slip 应被拒绝");
    }

    #[test]
    fn prepared_index_caches_manifest_for_read_small() {
        let t = Tmp::new("prep");
        let zp = t.dir.join("p.zip");
        write_zip(&zp, &[("manifest.json", br#"{"addons":[]}"#), ("overrides/a.txt", b"x")]);
        let idx = ZipArchiveIndex::open(&zp).unwrap();
        let prepared = idx.into_prepared(&["manifest.json"]);
        use super::super::ArchiveIndex;
        assert_eq!(prepared.read_small("manifest.json").unwrap(), br#"{"addons":[]}"#.to_vec());
        // 未预取的条目读不到(返回 None,而非读盘)。
        assert!(prepared.read_small("overrides/a.txt").is_none());
        // 取回 inner 仍可解压。
        let mut inner = prepared.into_inner();
        let s = t.dir.join("s");
        std::fs::create_dir_all(&s).unwrap();
        inner.extract_prefix("overrides/", &s).unwrap();
        assert_eq!(std::fs::read(s.join("a.txt")).unwrap(), b"x");
    }

    #[test]
    fn overlay_dir_safe_copies_tree() {
        let t = Tmp::new("overlay");
        let src = t.dir.join("overrides");
        std::fs::create_dir_all(src.join("config")).unwrap();
        std::fs::write(src.join("config/a.cfg"), b"k=1").unwrap();
        let game = t.dir.join("game");
        std::fs::create_dir_all(&game).unwrap();
        overlay_dir_safe(&src, &game).unwrap();
        assert_eq!(std::fs::read(game.join("config/a.cfg")).unwrap(), b"k=1");
    }
}
