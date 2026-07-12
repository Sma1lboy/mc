//! 崩溃日志分析。
//!
//! 扫描游戏的运行日志或崩溃报告（latest.log / hs_err / crash-report），
//! 用一组有优先级的关键词规则把晦涩的 Java 异常翻译成人话原因 + 可执行
//! 建议。设计参考 PCL 的 ModCrash 思路：内存 / Java 版本这类「环境根因」
//! 优先级最高，其次才是 mod 相关问题，最后才回退到泛型提示。
//!
//! 本模块是纯逻辑、无 I/O，便于在任意上层（CLI / Tauri）直接复用。

use serde::Serialize;

/// 崩溃类别：粗粒度归类，供上层 UI 做本地化标签与分组。
///
/// 这是「机器可读」的归类（`reason` 是给人看的一句话），上层据 [`slug`](CrashCategory::slug)
/// 映射到本地化的类别名。优先级与 [`RULES`] 的排序一致——环境根因在前。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CrashCategory {
    /// 内存不足（OutOfMemoryError）。
    OutOfMemory,
    /// 内存设置/位数问题（无法预留空间、堆大小无效）。
    Memory,
    /// Java 版本过低。
    JavaVersion,
    /// 加载器（Fabric / Forge）未正确安装。
    LoaderMissing,
    /// 缺少前置 mod / 依赖解析失败。
    MissingDependency,
    /// 重复 mod。
    DuplicateMod,
    /// mod 之间或与 MC 版本不兼容。
    IncompatibleMod,
    /// Mixin 注入失败。
    Mixin,
    /// 显卡 / OpenGL / 窗口创建问题。
    Graphics,
    /// mod 与 MC 版本不匹配（NoSuchMethod/Field）。
    VersionMismatch,
    /// 未命中具体规则的泛型异常退出。
    Unknown,
}

impl CrashCategory {
    /// 稳定的小写下划线 slug，作为前端本地化类别标签的键（`crash.cat.<slug>`）。
    pub fn slug(self) -> &'static str {
        match self {
            CrashCategory::OutOfMemory => "out_of_memory",
            CrashCategory::Memory => "memory",
            CrashCategory::JavaVersion => "java_version",
            CrashCategory::LoaderMissing => "loader_missing",
            CrashCategory::MissingDependency => "missing_dependency",
            CrashCategory::DuplicateMod => "duplicate_mod",
            CrashCategory::IncompatibleMod => "incompatible_mod",
            CrashCategory::Mixin => "mixin",
            CrashCategory::Graphics => "graphics",
            CrashCategory::VersionMismatch => "version_mismatch",
            CrashCategory::Unknown => "unknown",
        }
    }
}

/// 一次崩溃分析的结论。
#[derive(Debug, Clone, Serialize)]
pub struct CrashAnalysis {
    /// 机器可读的崩溃类别（供 UI 本地化标签 / 分组）。
    pub category: CrashCategory,
    /// 人话原因（一句话说明发生了什么）。
    pub reason: String,
    /// 给用户的可执行建议，按重要程度排序。
    pub suggestions: Vec<String>,
    /// 命中的原始日志行（截断到 200 字符），便于排查与展示证据。
    pub matched: Option<String>,
}

/// 单条规则：所有 `all` 关键词都出现，且 `any` 中至少一个出现（`any` 为空表示不约束）。
struct Rule {
    /// 必须全部出现的关键词。
    all: &'static [&'static str],
    /// 至少出现其一的关键词；为空切片表示不施加该约束。
    any: &'static [&'static str],
    /// 该规则所属的崩溃类别。
    category: CrashCategory,
    /// 人话原因。
    reason: &'static str,
    /// 建议列表。
    suggestions: &'static [&'static str],
}

/// 规则表，按优先级从高到低排列。
///
/// 优先级原则：环境根因（内存 / Java 版本）排在最前，因为它们往往是
/// 其他报错的真正源头；mod 相关问题居中；版本不匹配类（NoSuchMethod 等）
/// 放在较后，避免抢占更精确的命中。
const RULES: &[Rule] = &[
    // —— 内存不足 ——
    Rule {
        all: &["OutOfMemoryError"],
        any: &[],
        category: CrashCategory::OutOfMemory,
        reason: "内存不足：游戏申请的内存超出了已分配的上限。",
        suggestions: &[
            "调大最大内存（-Xmx），例如把分配内存提高到 4G 或更多。",
            "关闭其他占用内存的程序，给游戏腾出空间。",
            "如果安装了大量 mod 或高清材质，请相应增加分配内存。",
        ],
    },
    // —— 32 位 Java / 堆设置过大 ——
    // 注意：此规则需排在「Java 版本过低」之前，因为两者都可能出现 heap 字样，
    // 但「无法预留足够空间」是明确的 32 位/内存上限问题。
    Rule {
        all: &["Could not reserve enough space"],
        any: &[],
        category: CrashCategory::Memory,
        reason: "无法预留足够内存空间：通常是使用了 32 位 Java，或最大内存设置得过大。",
        suggestions: &[
            "改用 64 位 Java（32 位 Java 单进程最多只能用约 1.5G 内存）。",
            "把最大内存（-Xmx）调小到合理范围，或匹配你的物理内存。",
        ],
    },
    Rule {
        all: &["Invalid maximum heap size"],
        any: &[],
        category: CrashCategory::Memory,
        reason: "最大堆大小无效：内存设置过大或 Java 位数不支持该值。",
        suggestions: &[
            "改用 64 位 Java 以支持更大的内存设置。",
            "把最大内存（-Xmx）调小到合理范围。",
        ],
    },
    // —— Java 版本过低 ——
    Rule {
        all: &["UnsupportedClassVersionError"],
        any: &[],
        category: CrashCategory::JavaVersion,
        reason: "Java 版本过低：当前 Java 无法运行为更高版本编译的代码。",
        suggestions: &[
            "更换为更高版本的 Java（如新版 MC 需要 Java 17 或 21）。",
            "在启动器设置里指定正确的 Java 路径。",
        ],
    },
    Rule {
        all: &["has been compiled by a more recent version of the Java Runtime"],
        any: &[],
        category: CrashCategory::JavaVersion,
        reason: "Java 版本过低：有类是用更高版本的 Java 编译的，当前运行时不支持。",
        suggestions: &[
            "更换为更高版本的 Java（如新版 MC 需要 Java 17 或 21）。",
            "在启动器设置里指定正确的 Java 路径。",
        ],
    },
    // —— 加载器未正确安装 ——
    Rule {
        all: &["ClassNotFoundException", "net.fabricmc"],
        any: &[],
        category: CrashCategory::LoaderMissing,
        reason: "Fabric 加载器未正确安装：找不到 Fabric 的核心类。",
        suggestions: &[
            "重新安装 Fabric 加载器，并确认版本与 MC 匹配。",
            "确认你选择的是带 Fabric 的版本来启动。",
        ],
    },
    Rule {
        all: &["ClassNotFoundException", "net.minecraftforge"],
        any: &[],
        category: CrashCategory::LoaderMissing,
        reason: "Forge 加载器未正确安装：找不到 Forge 的核心类。",
        suggestions: &[
            "重新安装 Forge，并确认版本与 MC 匹配。",
            "确认你选择的是带 Forge 的版本来启动。",
        ],
    },
    // —— 缺少前置 mod / 依赖 ——
    // 放在 Mixin / 重复 mod 之前：依赖缺失常常是诱因，应优先提示。
    Rule {
        all: &["requires", "which is missing"],
        any: &[],
        category: CrashCategory::MissingDependency,
        reason: "缺少前置 mod：某个 mod 依赖另一个尚未安装的 mod。",
        suggestions: &[
            "根据日志里 “requires …” 的提示，安装缺少的前置 mod。",
            "确认所有 mod 的版本与 MC 及加载器版本一致。",
        ],
    },
    Rule {
        all: &["ModResolutionException"],
        any: &[],
        category: CrashCategory::MissingDependency,
        reason: "mod 依赖解析失败：缺少前置或版本不满足。",
        suggestions: &[
            "按日志提示补齐缺失的前置 mod。",
            "确认所有 mod 的版本与 MC 及加载器版本一致。",
        ],
    },
    Rule {
        all: &["Mod resolution"],
        any: &[],
        category: CrashCategory::MissingDependency,
        reason: "mod 依赖解析失败：缺少前置或版本不满足。",
        suggestions: &[
            "按日志提示补齐缺失的前置 mod。",
            "确认所有 mod 的版本与 MC 及加载器版本一致。",
        ],
    },
    // —— 重复 mod ——
    Rule {
        all: &["DuplicateModsFoundException"],
        any: &[],
        category: CrashCategory::DuplicateMod,
        reason: "存在重复的 mod：同一个 mod 安装了多个版本。",
        suggestions: &[
            "在 mods 文件夹里删除重复或多余版本的 mod，只保留一个。",
            "检查是否有 mod 被同时放进了实例目录和全局目录。",
        ],
    },
    Rule {
        all: &["Found a duplicate mod"],
        any: &[],
        category: CrashCategory::DuplicateMod,
        reason: "存在重复的 mod：同一个 mod 安装了多个版本。",
        suggestions: &[
            "在 mods 文件夹里删除重复或多余版本的 mod，只保留一个。",
            "检查是否有 mod 被同时放进了实例目录和全局目录。",
        ],
    },
    // —— mod 不兼容 ——
    Rule {
        all: &["Incompatible mod set"],
        any: &[],
        category: CrashCategory::IncompatibleMod,
        reason: "mod 集合不兼容：部分 mod 与当前 MC 或加载器版本冲突。",
        suggestions: &[
            "根据日志找出冲突的 mod，移除或更换为兼容版本。",
            "确认所有 mod 都对应同一 MC 版本和同一加载器。",
        ],
    },
    Rule {
        all: &["incompatible with"],
        any: &[],
        category: CrashCategory::IncompatibleMod,
        reason: "mod 不兼容：某个 mod 声明与当前 MC 或其他 mod 不兼容。",
        suggestions: &[
            "根据日志找出冲突的 mod，移除或更换为兼容版本。",
            "确认所有 mod 都对应同一 MC 版本和同一加载器。",
        ],
    },
    // —— Mixin 冲突 / 不兼容 ——
    Rule {
        all: &["Mixin"],
        any: &[
            "apply mixin",
            "FailedException",
            "InvalidMixinException",
        ],
        category: CrashCategory::Mixin,
        reason: "Mixin 注入失败：某个 mod 的 Mixin 与其他 mod 或 MC 版本冲突。",
        suggestions: &[
            "逐个排查并移除可疑 mod，定位引发 Mixin 冲突的那一个。",
            "把相关 mod 更新到与当前 MC 版本兼容的版本。",
            "确认没有同时安装两个功能冲突的 mod。",
        ],
    },
    // —— 显卡 / OpenGL 问题 ——
    Rule {
        all: &["Pixel format not accelerated"],
        any: &[],
        category: CrashCategory::Graphics,
        reason: "显卡未启用硬件加速：OpenGL 无法使用加速的像素格式。",
        suggestions: &[
            "更新显卡驱动到最新版本。",
            "确认游戏使用的是独立显卡而非核显（笔记本请在显卡控制面板里设置）。",
        ],
    },
    Rule {
        all: &["GLFW error"],
        any: &[],
        category: CrashCategory::Graphics,
        reason: "窗口/图形初始化失败（GLFW 错误）：通常是显卡驱动或 OpenGL 问题。",
        suggestions: &[
            "更新显卡驱动到最新版本。",
            "确认显卡支持游戏所需的 OpenGL 版本。",
        ],
    },
    Rule {
        all: &["Failed to create window"],
        any: &[],
        category: CrashCategory::Graphics,
        reason: "无法创建游戏窗口：多为显卡驱动或 OpenGL 支持不足。",
        suggestions: &[
            "更新显卡驱动到最新版本。",
            "确认游戏使用的是独立显卡，且支持所需 OpenGL 版本。",
        ],
    },
    Rule {
        all: &["OpenGL"],
        any: &[],
        category: CrashCategory::Graphics,
        reason: "OpenGL 相关错误：显卡驱动或 OpenGL 支持存在问题。",
        suggestions: &[
            "更新显卡驱动到最新版本。",
            "确认显卡支持游戏所需的 OpenGL 版本。",
        ],
    },
    // —— mod 与 MC 版本不匹配（放在较后，避免抢占更精确的命中）——
    Rule {
        all: &["NoSuchMethodError"],
        any: &[],
        category: CrashCategory::VersionMismatch,
        reason: "方法不存在（NoSuchMethodError）：通常是 mod 与 MC 版本不匹配。",
        suggestions: &[
            "把出错的 mod 更换为与当前 MC 版本对应的版本。",
            "确认所有 mod 都对应同一 MC 版本。",
        ],
    },
    Rule {
        all: &["NoSuchFieldError"],
        any: &[],
        category: CrashCategory::VersionMismatch,
        reason: "字段不存在（NoSuchFieldError）：通常是 mod 与 MC 版本不匹配。",
        suggestions: &[
            "把出错的 mod 更换为与当前 MC 版本对应的版本。",
            "确认所有 mod 都对应同一 MC 版本。",
        ],
    },
];

/// 把命中的原始日志行截断到 200 字符（按字符而非字节，避免切断 UTF-8）。
fn truncate_line(line: &str) -> String {
    const MAX: usize = 200;
    if line.chars().count() <= MAX {
        line.to_string()
    } else {
        line.chars().take(MAX).collect()
    }
}

/// 在日志中查找包含 `needle` 的第一行（用于回填 `matched`）。
fn find_line_containing<'a>(log: &'a str, needle: &str) -> Option<&'a str> {
    log.lines().find(|line| line.contains(needle))
}

/// 判断某条规则是否命中给定日志，命中则返回用于 `matched` 的 needle。
///
/// 返回的 needle 优先取 `any` 中实际命中的那个（更具体），否则取 `all` 的最后一个。
fn rule_matches(rule: &Rule, log: &str) -> Option<&'static str> {
    // 所有 all 关键词都必须出现。
    if !rule.all.iter().all(|kw| log.contains(kw)) {
        return None;
    }
    // any 约束：为空则跳过；否则至少命中一个。
    let any_hit = if rule.any.is_empty() {
        None
    } else {
        match rule.any.iter().find(|kw| log.contains(**kw)) {
            Some(kw) => Some(*kw),
            None => return None,
        }
    };
    // 优先用命中的 any 关键词定位原始行，其次用 all 的最后一个关键词。
    Some(any_hit.unwrap_or_else(|| rule.all.last().copied().unwrap_or("")))
}

/// 分析一段游戏日志 / 崩溃报告，命中规则则返回结论，否则返回 `None`。
///
/// 规则按优先级排列，返回第一个命中的（内存 / Java 版本类优先）。
pub fn analyze(log: &str) -> Option<CrashAnalysis> {
    for rule in RULES {
        if let Some(needle) = rule_matches(rule, log) {
            let matched = find_line_containing(log, needle).map(truncate_line);
            return Some(CrashAnalysis {
                category: rule.category,
                reason: rule.reason.to_string(),
                suggestions: rule.suggestions.iter().map(|s| s.to_string()).collect(),
                matched,
            });
        }
    }
    None
}

/// 结合退出码与日志做分析。
///
/// - `exit_code == 0`：视为正常退出，返回 `None`。
/// - 否则先跑 [`analyze`]；若无规则命中，回退一个泛型「异常退出」结论，
///   提示用户查看日志。
pub fn analyze_exit(exit_code: i32, log: &str) -> Option<CrashAnalysis> {
    if exit_code == 0 {
        return None;
    }
    if let Some(found) = analyze(log) {
        return Some(found);
    }
    Some(CrashAnalysis {
        category: CrashCategory::Unknown,
        reason: format!("游戏异常退出（代码 {exit_code}），请查看日志。"),
        suggestions: vec![
            "打开游戏日志（latest.log 或崩溃报告）查找具体报错。".to_string(),
            "尝试逐个移除 mod 以定位问题，或在干净版本上复现。".to_string(),
        ],
        matched: None,
    })
}

#[cfg(test)]
mod tests;
