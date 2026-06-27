//! 文件校验和工具。下载完成后必须校验 sha1，避免镜像/网络损坏的文件进入版本库。
//!
//! 设计要点:
//! - `sha1_file` 流式读取(8KiB 缓冲),不把整个文件载入内存,适配几百 MB 的客户端 jar。
//! - `verify_sha1` 故意吞掉所有错误返回 `false`:调用方只关心"这个文件是不是好的",
//!   文件缺失 / 读失败 / 不匹配语义上都等于"需要重新下载"。
//! - `find_broken` 用 rayon 并行扫描整批文件,因为磁盘 IO + 哈希在大版本上是瓶颈。

use std::fs::File;
use std::io::Read;
use std::path::Path;

use md5::Md5;
use rayon::prelude::*;
use sha1::{Digest, Sha1};
use sha2::Sha512;

use crate::error::{IoResultExt, Result};

use super::DownloadItem;

/// 用任意 [`Digest`] 算法流式读取文件并返回小写十六进制摘要。
///
/// 内存占用恒定(64KiB 缓冲),适配几百 MB 的客户端 jar。文件不存在 / 不可读会
/// 返回 [`crate::error::CoreError::Io`]。是 [`sha1_file`]/[`sha512_file`]/[`md5_file`] 的共同底座。
fn hash_file<D: Digest>(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_path(path)?;
    let mut hasher = D::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).with_path(path)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 读取文件并返回其 sha1 的小写十六进制字符串。原版 version json / 库用 sha1。
pub fn sha1_file(path: &Path) -> Result<String> {
    hash_file::<Sha1>(path)
}

/// 内存字节的 sha1(小写十六进制)。领域 overrides blob 的完整性/变更检测用。
pub fn sha1_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

/// 读取文件并返回其 sha512 的小写十六进制字符串。Modrinth `.mrpack` 索引以 sha512 为准。
pub fn sha512_file(path: &Path) -> Result<String> {
    hash_file::<Sha512>(path)
}

/// 读取文件并返回其 md5 的小写十六进制字符串。CurseForge 文件哈希之一。
pub fn md5_file(path: &Path) -> Result<String> {
    hash_file::<Md5>(path)
}

/// 校验 `path` 的 sha1 是否等于 `expected`(大小写不敏感)。
///
/// 文件不存在、读取失败或哈希不匹配一律返回 `false`,使调用方可以用单一布尔
/// 判断"是否需要(重新)下载"。
pub fn verify_sha1(path: &Path, expected: &str) -> bool {
    verify_with(sha1_file(path), expected)
}

/// 校验 `path` 的 sha512(大小写不敏感);失败 / 不存在均为 `false`。
pub fn verify_sha512(path: &Path, expected: &str) -> bool {
    verify_with(sha512_file(path), expected)
}

/// 校验 `path` 的 md5(大小写不敏感);失败 / 不存在均为 `false`。
pub fn verify_md5(path: &Path, expected: &str) -> bool {
    verify_with(md5_file(path), expected)
}

/// 把"算出的摘要(可能失败)"与"期望串"做大小写不敏感比较;算失败即 `false`。
fn verify_with(actual: Result<String>, expected: &str) -> bool {
    match actual {
        Ok(actual) => actual.eq_ignore_ascii_case(expected.trim()),
        Err(_) => false,
    }
}

/// 一个待校验的期望摘要,绑定了算法。整合包导入(mrpack=sha512、CurseForge=md5)与
/// 导出反查都需要按平台选取算法,而非硬编码 sha1。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checksum {
    Sha1(String),
    Sha512(String),
    Md5(String),
}

impl Checksum {
    /// 期望摘要的十六进制串。
    pub fn expected(&self) -> &str {
        match self {
            Checksum::Sha1(s) | Checksum::Sha512(s) | Checksum::Md5(s) => s,
        }
    }

    /// 按本枚举的算法校验 `path`;文件不存在 / 读取失败 / 不匹配一律 `false`。
    pub fn verify(&self, path: &Path) -> bool {
        match self {
            Checksum::Sha1(e) => verify_sha1(path, e),
            Checksum::Sha512(e) => verify_sha512(path, e),
            Checksum::Md5(e) => verify_md5(path, e),
        }
    }

    /// 从一组可选摘要里挑"最强且存在"的一个(sha512 > sha1 > md5),供下载后校验。
    /// 全为 `None` 时返回 `None`(无法校验,只能信任下载成功)。
    pub fn strongest(
        sha512: Option<&str>,
        sha1: Option<&str>,
        md5: Option<&str>,
    ) -> Option<Checksum> {
        if let Some(s) = sha512.filter(|s| !s.is_empty()) {
            Some(Checksum::Sha512(s.to_string()))
        } else if let Some(s) = sha1.filter(|s| !s.is_empty()) {
            Some(Checksum::Sha1(s.to_string()))
        } else {
            md5.filter(|s| !s.is_empty()).map(|s| Checksum::Md5(s.to_string()))
        }
    }
}

/// 并行扫描一批下载项,返回所有"坏项"的下标。
///
/// 坏项定义:目标文件不存在,或提供了 sha1 但实际不匹配。未提供 sha1 的项
/// 只要文件存在就视为完好(无法校验时信任其存在性,避免无谓重下)。
pub fn find_broken(items: &[DownloadItem]) -> Vec<usize> {
    let mut broken: Vec<usize> = items
        .par_iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            let ok = match item.checksum() {
                Some(c) => c.verify(&item.path),
                None => item.path.exists(),
            };
            if ok {
                None
            } else {
                Some(idx)
            }
        })
        .collect();
    // par_iter 的产出顺序不保证,排序后返回让结果可重现、便于测试与日志。
    broken.sort_unstable();
    broken
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mc-core-checksum-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(dir: &Path, name: &str, content: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        let mut f = File::create(&p).unwrap();
        f.write_all(content).unwrap();
        p
    }

    #[test]
    fn sha1_of_known_content() {
        let dir = tmp_dir();
        // sha1("abc") = a9993e364706816aba3e25717850c26c9cd0d89d
        let p = write_file(&dir, "abc.txt", b"abc");
        assert_eq!(sha1_file(&p).unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn sha512_and_md5_of_known_content() {
        let dir = tmp_dir();
        let p = write_file(&dir, "abc-multi.txt", b"abc");
        assert_eq!(
            sha512_file(&p).unwrap(),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assert_eq!(md5_file(&p).unwrap(), "900150983cd24fb0d6963f7d28e17f72");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn checksum_enum_verifies_each_algo_and_picks_strongest() {
        let dir = tmp_dir();
        let p = write_file(&dir, "abc-enum.txt", b"abc");

        assert!(Checksum::Sha1("a9993e364706816aba3e25717850c26c9cd0d89d".into()).verify(&p));
        assert!(Checksum::Md5("900150983cd24fb0d6963f7d28e17f72".into()).verify(&p));
        assert!(!Checksum::Md5("deadbeef".into()).verify(&p));

        // strongest:sha512 优先,其次 sha1,再次 md5;空串视为缺失。
        assert_eq!(
            Checksum::strongest(Some("aa"), Some("bb"), Some("cc")),
            Some(Checksum::Sha512("aa".into()))
        );
        assert_eq!(
            Checksum::strongest(Some(""), Some("bb"), Some("cc")),
            Some(Checksum::Sha1("bb".into()))
        );
        assert_eq!(Checksum::strongest(None, None, Some("cc")), Some(Checksum::Md5("cc".into())));
        assert_eq!(Checksum::strongest(None, None, None), None);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn verify_is_case_insensitive_and_tolerant() {
        let dir = tmp_dir();
        let p = write_file(&dir, "abc2.txt", b"abc");
        assert!(verify_sha1(&p, "A9993E364706816ABA3E25717850C26C9CD0D89D"));
        assert!(verify_sha1(&p, "a9993e364706816aba3e25717850c26c9cd0d89d"));
        assert!(!verify_sha1(&p, "deadbeef"));
        // 不存在的文件 -> false
        assert!(!verify_sha1(&dir.join("nope.txt"), "a9993e364706816aba3e25717850c26c9cd0d89d"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn find_broken_detects_missing_and_mismatched() {
        let dir = tmp_dir();
        let good = write_file(&dir, "good.bin", b"abc");
        let bad = write_file(&dir, "bad.bin", b"xyz");

        const ABC_SHA1: &str = "a9993e364706816aba3e25717850c26c9cd0d89d";
        let items = vec![
            // 0: 正确 sha1 -> 完好
            DownloadItem::new("http://x/good", good.clone(), Some(ABC_SHA1.into()), None),
            // 1: sha1 不匹配 -> 坏
            DownloadItem::new("http://x/bad", bad.clone(), Some(ABC_SHA1.into()), None),
            // 2: 文件不存在 -> 坏
            DownloadItem::new("http://x/missing", dir.join("missing.bin"), Some(ABC_SHA1.into()), None),
            // 3: 无 sha1 但文件存在 -> 完好
            DownloadItem::new("http://x/good", good.clone(), None, None),
        ];

        assert_eq!(find_broken(&items), vec![1, 2]);
        std::fs::remove_file(&good).ok();
        std::fs::remove_file(&bad).ok();
    }
}
