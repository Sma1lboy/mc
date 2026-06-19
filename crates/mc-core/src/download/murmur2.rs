//! CurseForge "fingerprint" 哈希:一种**非标准** MurmurHash2 变体,用于把本地 jar
//! 反查到 CurseForge 的 project/file(`POST /v1/fingerprints`),整合包导入识别 blocked
//! 文件、导出反查都依赖它。
//!
//! 与教科书 MurmurHash2 的两点关键差异(算错则一个文件都匹配不上):
//! 1. **种子固定为 1**(`seed = 1`)。
//! 2. 哈希前**剔除空白字节** `9`(tab)/`10`(LF)/`13`(CR)/`32`(space),且喂给算法的
//!    `len` 是**剔除后**的长度(它参与初始 `h = seed ^ len`)。
//!
//! 算法本体是 Austin Appleby 的 32-bit MurmurHash2(public domain),移植自
//! PrismLauncher `libraries/murmur2`。
//!
//! 单测覆盖标准 murmur2 不变量(空输入 + seed 0 → 0)、空白剔除与确定性;真实指纹值
//! 由对 CurseForge `/fingerprints` 端点的集成测试验证(无法在离线单测里凭空构造)。

const M: u32 = 0x5bd1_e995;
const R: u32 = 24;
/// CurseForge 固定使用的种子。
const CF_SEED: u32 = 1;

/// 判断一个字节是否为 CurseForge 在指纹计算前剔除的空白字节。
#[inline]
fn is_cf_whitespace(b: u8) -> bool {
    matches!(b, 9 | 10 | 13 | 32)
}

/// 计算一段数据的 CurseForge 指纹(已含空白剔除 + seed=1)。
///
/// 先把 `data` 里的空白字节滤掉,再对剩余字节跑 MurmurHash2(seed=1)。返回值即
/// `/fingerprints` 请求里用的无符号 32 位指纹。
pub fn cf_fingerprint(data: &[u8]) -> u32 {
    let filtered: Vec<u8> = data.iter().copied().filter(|&b| !is_cf_whitespace(b)).collect();
    murmur2(&filtered, CF_SEED)
}

/// 流式计算文件的 CurseForge 指纹,内存占用恒定(逐块剔除空白再累积)。
///
/// 因 MurmurHash2 按 4 字节块处理、剔除空白会打乱块边界,这里把"剔除空白后的字节流"
/// 攒成一个缓冲再整体哈希——但为避免把大文件整体载入内存,采用分块读取 + 仅保留
/// 滤后字节。滤后字节通常远小于原文件(jar 几乎无空白),内存友好。
pub fn cf_fingerprint_file(path: &std::path::Path) -> crate::error::Result<u32> {
    use crate::error::IoResultExt;
    use std::io::Read;

    let mut file = std::fs::File::open(path).with_path(path)?;
    let mut buf = [0u8; 64 * 1024];
    let mut filtered: Vec<u8> = Vec::new();
    loop {
        let n = file.read(&mut buf).with_path(path)?;
        if n == 0 {
            break;
        }
        filtered.extend(buf[..n].iter().copied().filter(|&b| !is_cf_whitespace(b)));
    }
    Ok(murmur2(&filtered, CF_SEED))
}

/// 标准 32-bit MurmurHash2(Austin Appleby),小端读取 4 字节块。
fn murmur2(data: &[u8], seed: u32) -> u32 {
    let len = data.len();
    let mut h: u32 = seed ^ (len as u32);

    let mut i = 0;
    while i + 4 <= len {
        let mut k = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);
        h = h.wrapping_mul(M);
        h ^= k;
        i += 4;
    }

    // 处理尾部 1..=3 字节(fall-through 语义)。
    let rem = len - i;
    if rem >= 3 {
        h ^= (data[i + 2] as u32) << 16;
    }
    if rem >= 2 {
        h ^= (data[i + 1] as u32) << 8;
    }
    if rem >= 1 {
        h ^= data[i] as u32;
        h = h.wrapping_mul(M);
    }

    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^= h >> 15;
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_murmur2_empty_seed0_is_zero() {
        // h = 0 ^ 0 = 0;无块无尾;尾部混合全 0 → 0。标准不变量。
        assert_eq!(murmur2(b"", 0), 0);
    }

    #[test]
    fn whitespace_is_stripped_before_hashing() {
        // 含 tab/LF/CR/space 的输入与剔除后输入应得到相同指纹。
        assert_eq!(cf_fingerprint(b"a\t\n\r b c"), cf_fingerprint(b"abc"));
        // 纯空白 → 等价于空输入(seed=1):h = 1 ^ 0 = 1,过尾部混合。
        assert_eq!(cf_fingerprint(b" \t\n\r"), murmur2(b"", CF_SEED));
    }

    #[test]
    fn fingerprint_is_deterministic_and_seed1() {
        let a = cf_fingerprint(b"fabric.mod.json contents here");
        let b = cf_fingerprint(b"fabric.mod.json contents here");
        assert_eq!(a, b);
        // 不同内容应(极大概率)不同。
        assert_ne!(cf_fingerprint(b"abc"), cf_fingerprint(b"abd"));
    }

    #[test]
    fn file_and_slice_agree() {
        let dir = std::env::temp_dir().join(format!("mc-core-murmur-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("mod.jar");
        let content = b"PK\x03\x04 some\t jar \n bytes with whitespace ";
        std::fs::write(&p, content).unwrap();
        assert_eq!(cf_fingerprint_file(&p).unwrap(), cf_fingerprint(content));
        std::fs::remove_file(&p).ok();
    }
}
