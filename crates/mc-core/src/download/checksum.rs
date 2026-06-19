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

use rayon::prelude::*;
use sha1::{Digest, Sha1};

use crate::error::{IoResultExt, Result};

use super::DownloadItem;

/// 读取文件并返回其 sha1 的小写十六进制字符串。
///
/// 流式读取,内存占用恒定。文件不存在 / 不可读会返回 [`crate::error::CoreError::Io`]。
pub fn sha1_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_path(path)?;
    let mut hasher = Sha1::new();
    // 64KiB 缓冲在吞吐与系统调用次数之间取折中。
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

/// 校验 `path` 的 sha1 是否等于 `expected`(大小写不敏感)。
///
/// 文件不存在、读取失败或哈希不匹配一律返回 `false`,使调用方可以用单一布尔
/// 判断"是否需要(重新)下载"。
pub fn verify_sha1(path: &Path, expected: &str) -> bool {
    match sha1_file(path) {
        Ok(actual) => actual.eq_ignore_ascii_case(expected.trim()),
        Err(_) => false,
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
            let ok = match &item.sha1 {
                Some(expected) => verify_sha1(&item.path, expected),
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

        let items = vec![
            // 0: 正确 sha1 -> 完好
            DownloadItem {
                url: "http://x/good".into(),
                path: good.clone(),
                sha1: Some("a9993e364706816aba3e25717850c26c9cd0d89d".into()),
                size: None,
            },
            // 1: sha1 不匹配 -> 坏
            DownloadItem {
                url: "http://x/bad".into(),
                path: bad.clone(),
                sha1: Some("a9993e364706816aba3e25717850c26c9cd0d89d".into()),
                size: None,
            },
            // 2: 文件不存在 -> 坏
            DownloadItem {
                url: "http://x/missing".into(),
                path: dir.join("missing.bin"),
                sha1: Some("a9993e364706816aba3e25717850c26c9cd0d89d".into()),
                size: None,
            },
            // 3: 无 sha1 但文件存在 -> 完好
            DownloadItem { url: "http://x/good".into(), path: good.clone(), sha1: None, size: None },
        ];

        assert_eq!(find_broken(&items), vec![1, 2]);
        std::fs::remove_file(&good).ok();
        std::fs::remove_file(&bad).ok();
    }
}
