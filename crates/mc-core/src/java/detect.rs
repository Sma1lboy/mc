//! 探测系统上已安装的 Java。
//!
//! 设计要点:
//!   - `probe` 跑一次 `<path> -version` (输出在 stderr), 解析版本与位数, 失败返回 `None`。
//!   - `detect_all` 汇总一批候选可执行文件路径 (JAVA_HOME、PATH、各平台常见安装目录),
//!     去重后**并发**探测, 忽略失败项。
//!
//! 我们不依赖外部 `which` 程序: 直接遍历 `PATH` 环境变量自己找 `java`/`java.exe`,
//! 这样跨平台且无额外依赖。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use futures::future::join_all;
use tokio::process::Command;

use super::version::JavaVersion;
use super::{java_exe_name, JavaInstall};

/// 在某个具体路径上运行 `java -version` 并解析结果。
///
/// `java -version` 把信息写到 **stderr**; 我们合并 stdout+stderr 一起解析以防个别
/// JVM 写到 stdout。解析不出版本、或进程启动失败时返回 `None`。
pub async fn probe(path: &Path) -> Option<JavaInstall> {
    probe_with_source(path, "system").await
}

/// 同 [`probe`], 但允许调用方标注来源 (PATH/JAVA_HOME/system…)。
pub async fn probe_with_source(path: &Path, source: &str) -> Option<JavaInstall> {
    let output = Command::new(path).arg("-version").output().await.ok()?;

    // 绝大多数 JVM 把版本写到 stderr, 但稳妥起见两个流都看。
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.stdout.is_empty() {
        text.push('\n');
        text.push_str(&String::from_utf8_lossy(&output.stdout));
    }

    let version = JavaVersion::parse_from_output(&text)?;
    let is_64bit = infer_64bit(&text);

    // 规范化路径, 便于上层比较/去重 (失败则退回原路径)。
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    Some(JavaInstall {
        path: canonical,
        version,
        is_64bit,
        source: source.to_string(),
    })
}

/// 从 `java -version` 文本里推断是否 64 位。
///
/// 64 位 JVM 通常打印 `64-Bit Server VM`; 个别只标 `64-Bit`。明确出现 32 位字样时
/// 判为 32 位。两者都没有时, 现代 JDK 几乎全是 64 位, 默认 `true`。
fn infer_64bit(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("64-bit") {
        return true;
    }
    if lower.contains("32-bit") || (lower.contains("client vm") && lower.contains("x86")) {
        return false;
    }
    // 取不到位数信息时默认 64 位 (与任务要求一致)。
    true
}

/// 探测系统上所有能找到的 Java 安装。
///
/// 候选来源:
///   1. `JAVA_HOME/bin/java`
///   2. `PATH` 中每个目录下的 `java` (Windows 为 `java.exe`)
///   3. 各平台常见安装目录的 glob 展开
///
/// 按规范化路径去重后并发 `probe`, 丢弃探测失败的项。
pub async fn detect_all() -> Vec<JavaInstall> {
    // 每个候选携带其来源标签, 以便结果里区分 PATH/JAVA_HOME/system。
    let mut candidates: Vec<(PathBuf, &'static str)> = Vec::new();

    // 1) JAVA_HOME
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let exe = PathBuf::from(home).join("bin").join(java_exe_name());
        candidates.push((exe, "JAVA_HOME"));
    }

    // 2) PATH 各目录
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let exe = dir.join(java_exe_name());
            candidates.push((exe, "PATH"));
        }
    }

    // 3) 各平台常见安装目录
    for exe in well_known_installs() {
        candidates.push((exe, "system"));
    }

    // 去重: 先按"存在 + 规范化路径"过滤, 避免对同一二进制重复 probe。
    // 注意此处的规范化仅用于去重 key; probe 内部还会再次规范化写入结果。
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut unique: Vec<(PathBuf, &'static str)> = Vec::new();
    for (exe, source) in candidates {
        if !exe.exists() {
            continue;
        }
        let key = std::fs::canonicalize(&exe).unwrap_or_else(|_| exe.clone());
        if seen.insert(key) {
            unique.push((exe, source));
        }
    }

    // 并发 probe 全部候选。
    let probes = unique
        .into_iter()
        .map(|(exe, source)| async move { probe_with_source(&exe, source).await });
    let results = join_all(probes).await;

    // 过滤掉失败项; 同时再按规范化后的最终路径去重一次, 因为不同候选可能经符号链接
    // 指向同一真实二进制 (例如 PATH 里的 java 与 JAVA_HOME 实为一处)。
    let mut final_seen: HashSet<PathBuf> = HashSet::new();
    let mut installs: Vec<JavaInstall> = Vec::new();
    for inst in results.into_iter().flatten() {
        if final_seen.insert(inst.path.clone()) {
            installs.push(inst);
        }
    }
    installs
}

/// 展开各平台常见的 JDK 安装目录, 返回其中所有候选 `.../bin/java[.exe]`。
///
/// 这里手写 glob (只支持一层 `*`), 避免引入额外依赖。
fn well_known_installs() -> Vec<PathBuf> {
    let mut out = Vec::new();

    #[cfg(target_os = "macos")]
    {
        // 系统级与用户级 JavaVirtualMachines。
        out.extend(glob_bin_java(
            Path::new("/Library/Java/JavaVirtualMachines"),
            &["Contents", "Home", "bin"],
        ));
        if let Some(home) = dirs::home_dir() {
            out.extend(glob_bin_java(
                &home.join("Library/Java/JavaVirtualMachines"),
                &["Contents", "Home", "bin"],
            ));
        }
    }

    #[cfg(target_os = "linux")]
    {
        out.extend(glob_bin_java(Path::new("/usr/lib/jvm"), &["bin"]));
    }

    #[cfg(target_os = "windows")]
    {
        out.extend(glob_bin_java(
            Path::new(r"C:\Program Files\Java"),
            &["bin"],
        ));
        out.extend(glob_bin_java(
            Path::new(r"C:\Program Files\Eclipse Adoptium"),
            &["bin"],
        ));
    }

    out
}

/// 对 `base` 下的每个直接子目录 (即 glob 的 `*`), 拼接 `sub_dirs` 再加上 java 可执行名,
/// 收集所有真实存在的候选路径。
///
/// 例如 base=`/usr/lib/jvm`, sub_dirs=`["bin"]`
///   →  `/usr/lib/jvm/<each>/bin/java`
fn glob_bin_java(base: &Path, sub_dirs: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return out, // 目录不存在 / 无权限: 安静跳过。
    };
    for entry in entries.flatten() {
        let mut p = entry.path();
        if !p.is_dir() {
            continue;
        }
        for sub in sub_dirs {
            p = p.join(sub);
        }
        let exe = p.join(java_exe_name());
        if exe.exists() {
            out.push(exe);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_64bit_from_server_vm() {
        let text = "OpenJDK 64-Bit Server VM Temurin-17.0.14+7 (build 17.0.14+7, mixed mode)";
        assert!(infer_64bit(text));
    }

    #[test]
    fn infers_32bit_when_marked() {
        let text = "Java HotSpot(TM) Client VM (build 25.391-b13, 32-Bit, mixed mode)";
        assert!(!infer_64bit(text));
    }

    #[test]
    fn defaults_to_64bit_when_unknown() {
        // 没有任何位数线索时默认 64 位。
        assert!(infer_64bit("openjdk version \"17.0.14\""));
    }

    #[test]
    fn glob_returns_empty_for_missing_dir() {
        let out = glob_bin_java(Path::new("/definitely/not/a/real/jvm/dir/xyz"), &["bin"]);
        assert!(out.is_empty());
    }
}
