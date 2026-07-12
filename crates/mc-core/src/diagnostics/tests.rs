    use super::*;

    #[test]
    fn detects_out_of_memory() {
        let log = "Exception in thread \"main\" java.lang.OutOfMemoryError: Java heap space";
        let a = analyze(log).expect("应命中内存不足");
        assert!(a.reason.contains("内存不足"));
        assert_eq!(a.category, CrashCategory::OutOfMemory);
        assert_eq!(a.category.slug(), "out_of_memory");
        assert!(!a.suggestions.is_empty());
        assert!(a.matched.unwrap().contains("OutOfMemoryError"));
    }

    #[test]
    fn generic_fallback_is_unknown_category() {
        let a = analyze_exit(-1, "nothing interesting").expect("应有泛型结论");
        assert_eq!(a.category, CrashCategory::Unknown);
        assert_eq!(a.category.slug(), "unknown");
    }

    #[test]
    fn category_serializes() {
        let a = analyze("java.lang.NoSuchMethodError: foo").expect("应命中");
        assert_eq!(a.category, CrashCategory::VersionMismatch);
        let json = serde_json::to_string(&a).expect("应可序列化");
        assert!(json.contains("category"));
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
