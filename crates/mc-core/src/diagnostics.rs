//! 崩溃日志分析。
//!
//! 扫描游戏的运行日志或崩溃报告（latest.log / hs_err / crash-report），
//! 用一组有优先级的关键词规则把晦涩的 Java 异常翻译成人话原因 + 可执行
//! 建议。设计参考 PCL 的 ModCrash 思路：内存 / Java 版本这类「环境根因」
//! 优先级最高，其次才是 mod 相关问题，最后才回退到泛型提示。
//!
//! 本模块是纯逻辑、无 I/O，便于在任意上层（CLI / Tauri）直接复用。

use serde::Serialize;

/// 一次崩溃分析的结论。
#[derive(Debug, Clone, Serialize)]
pub struct CrashAnalysis {
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
        reason: "无法预留足够内存空间：通常是使用了 32 位 Java，或最大内存设置得过大。",
        suggestions: &[
            "改用 64 位 Java（32 位 Java 单进程最多只能用约 1.5G 内存）。",
            "把最大内存（-Xmx）调小到合理范围，或匹配你的物理内存。",
        ],
    },
    Rule {
        all: &["Invalid maximum heap size"],
        any: &[],
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
        reason: "Java 版本过低：当前 Java 无法运行为更高版本编译的代码。",
        suggestions: &[
            "更换为更高版本的 Java（如新版 MC 需要 Java 17 或 21）。",
            "在启动器设置里指定正确的 Java 路径。",
        ],
    },
    Rule {
        all: &["has been compiled by a more recent version of the Java Runtime"],
        any: &[],
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
        reason: "Fabric 加载器未正确安装：找不到 Fabric 的核心类。",
        suggestions: &[
            "重新安装 Fabric 加载器，并确认版本与 MC 匹配。",
            "确认你选择的是带 Fabric 的版本来启动。",
        ],
    },
    Rule {
        all: &["ClassNotFoundException", "net.minecraftforge"],
        any: &[],
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
        reason: "缺少前置 mod：某个 mod 依赖另一个尚未安装的 mod。",
        suggestions: &[
            "根据日志里 “requires …” 的提示，安装缺少的前置 mod。",
            "确认所有 mod 的版本与 MC 及加载器版本一致。",
        ],
    },
    Rule {
        all: &["ModResolutionException"],
        any: &[],
        reason: "mod 依赖解析失败：缺少前置或版本不满足。",
        suggestions: &[
            "按日志提示补齐缺失的前置 mod。",
            "确认所有 mod 的版本与 MC 及加载器版本一致。",
        ],
    },
    Rule {
        all: &["Mod resolution"],
        any: &[],
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
        reason: "存在重复的 mod：同一个 mod 安装了多个版本。",
        suggestions: &[
            "在 mods 文件夹里删除重复或多余版本的 mod，只保留一个。",
            "检查是否有 mod 被同时放进了实例目录和全局目录。",
        ],
    },
    Rule {
        all: &["Found a duplicate mod"],
        any: &[],
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
        reason: "mod 集合不兼容：部分 mod 与当前 MC 或加载器版本冲突。",
        suggestions: &[
            "根据日志找出冲突的 mod，移除或更换为兼容版本。",
            "确认所有 mod 都对应同一 MC 版本和同一加载器。",
        ],
    },
    Rule {
        all: &["incompatible with"],
        any: &[],
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
        reason: "显卡未启用硬件加速：OpenGL 无法使用加速的像素格式。",
        suggestions: &[
            "更新显卡驱动到最新版本。",
            "确认游戏使用的是独立显卡而非核显（笔记本请在显卡控制面板里设置）。",
        ],
    },
    Rule {
        all: &["GLFW error"],
        any: &[],
        reason: "窗口/图形初始化失败（GLFW 错误）：通常是显卡驱动或 OpenGL 问题。",
        suggestions: &[
            "更新显卡驱动到最新版本。",
            "确认显卡支持游戏所需的 OpenGL 版本。",
        ],
    },
    Rule {
        all: &["Failed to create window"],
        any: &[],
        reason: "无法创建游戏窗口：多为显卡驱动或 OpenGL 支持不足。",
        suggestions: &[
            "更新显卡驱动到最新版本。",
            "确认游戏使用的是独立显卡，且支持所需 OpenGL 版本。",
        ],
    },
    Rule {
        all: &["OpenGL"],
        any: &[],
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
        reason: "方法不存在（NoSuchMethodError）：通常是 mod 与 MC 版本不匹配。",
        suggestions: &[
            "把出错的 mod 更换为与当前 MC 版本对应的版本。",
            "确认所有 mod 都对应同一 MC 版本。",
        ],
    },
    Rule {
        all: &["NoSuchFieldError"],
        any: &[],
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
        reason: format!("游戏异常退出（代码 {exit_code}），请查看日志。"),
        suggestions: vec![
            "打开游戏日志（latest.log 或崩溃报告）查找具体报错。".to_string(),
            "尝试逐个移除 mod 以定位问题，或在干净版本上复现。".to_string(),
        ],
        matched: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_out_of_memory() {
        let log = "Exception in thread \"main\" java.lang.OutOfMemoryError: Java heap space";
        let a = analyze(log).expect("应命中内存不足");
        assert!(a.reason.contains("内存不足"));
        assert!(!a.suggestions.is_empty());
        assert!(a.matched.unwrap().contains("OutOfMemoryError"));
    }

    #[test]
    fn detects_reserve_space_32bit() {
        let log = "Error occurred during initialization of VM\nCould not reserve enough space for 2097152KB object heap";
        let a = analyze(log).expect("应命中 32 位/内存过大");
        assert!(a.reason.contains("无法预留足够内存空间"));
    }

    #[test]
    fn detects_invalid_max_heap() {
        let log = "Invalid maximum heap size: -Xmx8G";
        let a = analyze(log).expect("应命中堆大小无效");
        assert!(a.reason.contains("最大堆大小无效"));
    }

    #[test]
    fn detects_unsupported_class_version() {
        let log = "java.lang.UnsupportedClassVersionError: net/minecraft/client/main/Main has been compiled by a more recent version of the Java Runtime (class file version 65.0)";
        let a = analyze(log).expect("应命中 Java 版本过低");
        assert!(a.reason.contains("Java 版本过低"));
    }

    #[test]
    fn detects_compiled_by_more_recent_runtime() {
        // 没有 UnsupportedClassVersionError，仅有 "compiled by a more recent" 文案。
        let log = "class has been compiled by a more recent version of the Java Runtime";
        let a = analyze(log).expect("应命中 Java 版本过低");
        assert!(a.reason.contains("Java 版本过低"));
    }

    #[test]
    fn detects_fabric_loader_missing() {
        let log = "Caused by: java.lang.ClassNotFoundException: net.fabricmc.loader.impl.launch.knot.KnotClient";
        let a = analyze(log).expect("应命中 Fabric 未安装");
        assert!(a.reason.contains("Fabric"));
        assert!(a.reason.contains("未正确安装"));
    }

    #[test]
    fn detects_forge_loader_missing() {
        let log = "java.lang.ClassNotFoundException: net.minecraftforge.fml.common.Mod";
        let a = analyze(log).expect("应命中 Forge 未安装");
        assert!(a.reason.contains("Forge"));
    }

    #[test]
    fn detects_mixin_failure() {
        let log = "org.spongepowered.asm.mixin.injection.throwables.InvalidMixinException: Critical injection failure: could not apply mixin";
        let a = analyze(log).expect("应命中 Mixin 冲突");
        assert!(a.reason.contains("Mixin"));
    }

    #[test]
    fn detects_opengl_pixel_format() {
        let log = "org.lwjgl.LWJGLException: Pixel format not accelerated";
        let a = analyze(log).expect("应命中显卡/OpenGL");
        assert!(a.reason.contains("硬件加速") || a.reason.contains("显卡"));
    }

    #[test]
    fn detects_glfw_error() {
        let log = "GLFW error 65542: WGL: The driver does not appear to support OpenGL";
        let a = analyze(log).expect("应命中 GLFW/显卡");
        assert!(a.reason.contains("GLFW") || a.reason.contains("显卡"));
    }

    #[test]
    fn detects_failed_to_create_window() {
        let log = "Failed to create window";
        let a = analyze(log).expect("应命中无法创建窗口");
        assert!(a.reason.contains("窗口"));
    }

    #[test]
    fn detects_duplicate_mods() {
        let log = "net.fabricmc.loader.impl.discovery.DuplicateModsFoundException: Duplicate mods found";
        let a = analyze(log).expect("应命中重复 mod");
        assert!(a.reason.contains("重复"));
    }

    #[test]
    fn detects_found_a_duplicate_mod() {
        let log = "Found a duplicate mod sodium in the folder";
        let a = analyze(log).expect("应命中重复 mod");
        assert!(a.reason.contains("重复"));
    }

    #[test]
    fn detects_missing_dependency_requires() {
        let log = "Mod 'Example' (example) requires version 1.0 of fabric-api, which is missing!";
        let a = analyze(log).expect("应命中缺少前置");
        assert!(a.reason.contains("缺少前置"));
    }

    #[test]
    fn detects_mod_resolution_exception() {
        let log = "net.fabricmc.loader.impl.FormattedException: ModResolutionException: some failure";
        let a = analyze(log).expect("应命中依赖解析失败");
        assert!(a.reason.contains("依赖解析失败"));
    }

    #[test]
    fn detects_incompatible_mod_set() {
        let log = "Incompatible mod set! the following mods cannot be loaded together";
        let a = analyze(log).expect("应命中 mod 不兼容");
        assert!(a.reason.contains("不兼容"));
    }

    #[test]
    fn detects_incompatible_with() {
        let log = "Mod foo is incompatible with bar";
        let a = analyze(log).expect("应命中 mod 不兼容");
        assert!(a.reason.contains("不兼容"));
    }

    #[test]
    fn detects_no_such_method_error() {
        let log = "java.lang.NoSuchMethodError: net.minecraft.class_1234.method_5678()";
        let a = analyze(log).expect("应命中版本不匹配");
        assert!(a.reason.contains("方法不存在"));
        assert!(a.reason.contains("版本不匹配"));
    }

    #[test]
    fn detects_no_such_field_error() {
        let log = "java.lang.NoSuchFieldError: field_9999";
        let a = analyze(log).expect("应命中版本不匹配");
        assert!(a.reason.contains("字段不存在"));
    }

    #[test]
    fn no_match_returns_none() {
        let log = "[12:00:00] [main/INFO]: Setting user: Player\n[12:00:01] [main/INFO]: Stopping!";
        assert!(analyze(log).is_none());
    }

    #[test]
    fn priority_oom_before_generic_window() {
        // 同时出现内存不足与窗口失败，应优先返回内存不足（更高优先级）。
        let log = "Failed to create window\njava.lang.OutOfMemoryError: Java heap space";
        let a = analyze(log).expect("应命中");
        assert!(a.reason.contains("内存不足"));
    }

    #[test]
    fn priority_dependency_before_mixin() {
        // 缺少前置常是 Mixin 报错的诱因，应优先提示缺少前置。
        let log = "requires fabric-api, which is missing\nMixin apply mixin failed";
        let a = analyze(log).expect("应命中");
        assert!(a.reason.contains("缺少前置"));
    }

    #[test]
    fn matched_line_truncated_to_200_chars() {
        let long = "x".repeat(300);
        let log = format!("java.lang.OutOfMemoryError: {long}");
        let a = analyze(&log).expect("应命中");
        let matched = a.matched.expect("应有 matched");
        assert_eq!(matched.chars().count(), 200);
    }

    #[test]
    fn exit_zero_is_none() {
        assert!(analyze_exit(0, "java.lang.OutOfMemoryError").is_none());
    }

    #[test]
    fn exit_nonzero_uses_analyze_when_matched() {
        let a = analyze_exit(1, "java.lang.OutOfMemoryError: Java heap space")
            .expect("非零退出应有结论");
        assert!(a.reason.contains("内存不足"));
    }

    #[test]
    fn exit_nonzero_generic_fallback() {
        let a = analyze_exit(-1, "nothing interesting here").expect("应有泛型结论");
        assert!(a.reason.contains("异常退出"));
        assert!(a.reason.contains("-1"));
        assert!(a.matched.is_none());
        assert!(!a.suggestions.is_empty());
    }

    #[test]
    fn serializes_to_json() {
        // 确认 #[derive(Serialize)] 可用，供上层透传给前端。
        let a = analyze("java.lang.OutOfMemoryError").expect("应命中");
        let json = serde_json::to_string(&a).expect("应可序列化");
        assert!(json.contains("reason"));
        assert!(json.contains("suggestions"));
    }
}
