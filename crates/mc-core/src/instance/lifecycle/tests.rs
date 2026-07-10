    use super::*;
    use crate::modpack::formats::mrpack::MrpackIndex;
    use std::fs;
    use std::io::{Read, Write};
    use std::path::PathBuf;

    /// 临时 game root,Drop 时自动清理。
    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("mc-core-lifecycle-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn paths(&self) -> GamePaths {
            GamePaths::new(self.path.clone())
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    // ---- create_instance helpers ----

    #[test]
    fn slugify_instance_id_cleans_and_keeps_unicode() {
        assert_eq!(slugify_instance_id("My Pack 1.20"), "My-Pack-1.20");
        assert_eq!(slugify_instance_id("  weird/\\:name? "), "weird-name");
        assert_eq!(slugify_instance_id("我的整合包"), "我的整合包"); // 保留中文
        assert_eq!(slugify_instance_id("///"), "instance"); // 空结果回退
        assert_eq!(slugify_instance_id("a   b"), "a-b"); // 空白归一
    }

    #[test]
    fn unique_instance_id_suffixes_on_collision() {
        let root = TempRoot::new("unique");
        let paths = root.paths();
        assert_eq!(unique_instance_id(&paths, "Pack"), "Pack");
        fs::create_dir_all(paths.version_dir("Pack")).unwrap();
        assert_eq!(unique_instance_id(&paths, "Pack"), "Pack-2");
        fs::create_dir_all(paths.version_dir("Pack-2")).unwrap();
        assert_eq!(unique_instance_id(&paths, "Pack"), "Pack-3");
    }

    // ---- copy_instance ----

    #[test]
    fn copy_instance_rewrites_id_and_renames_files() {
        let root = TempRoot::new("copy");
        let paths = root.paths();

        // 造一个源实例:版本 json + jar + 一个 mod + instance.json。
        let src_dir = paths.version_dir("1.20.1");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            paths.version_json("1.20.1"),
            r#"{"id":"1.20.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )
        .unwrap();
        fs::write(paths.version_jar("1.20.1"), b"FAKEJAR").unwrap();
        fs::create_dir_all(src_dir.join("mods")).unwrap();
        fs::write(src_dir.join("mods/sodium.jar"), b"MODBYTES").unwrap();
        fs::write(src_dir.join("instance.json"), r#"{"name":"Source","memory_mb":4096}"#).unwrap();

        copy_instance(&paths, "1.20.1", "my-copy").unwrap();

        let dst_dir = paths.version_dir("my-copy");
        // 新名 json 存在、旧名 json 不存在。
        let new_json = paths.version_json("my-copy");
        assert!(new_json.is_file(), "应生成 my-copy.json");
        assert!(!dst_dir.join("1.20.1.json").exists(), "旧名 json 应被删除");

        // json 内部 id 已改写,其余字段保留。
        let parsed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&new_json).unwrap()).unwrap();
        assert_eq!(parsed["id"], "my-copy");
        assert_eq!(parsed["mainClass"], "net.minecraft.client.main.Main");
        assert_eq!(parsed["type"], "release");

        // jar 随 id 改名。
        assert!(paths.version_jar("my-copy").is_file(), "jar 应改名为 my-copy.jar");
        assert!(!dst_dir.join("1.20.1.jar").exists());

        // 游戏数据 + instance.json 被复制。
        assert_eq!(fs::read(dst_dir.join("mods/sodium.jar")).unwrap(), b"MODBYTES");
        assert!(dst_dir.join("instance.json").is_file());

        // 源实例保持原样(复制而非移动)。
        assert!(paths.version_json("1.20.1").is_file());
        assert!(paths.version_jar("1.20.1").is_file());
    }

    #[test]
    fn copy_instance_named_uniquifies_id_and_rewrites_name() {
        let root = TempRoot::new("copy-named");
        let paths = root.paths();

        fs::create_dir_all(paths.version_dir("1.20.1")).unwrap();
        fs::write(paths.version_json("1.20.1"), r#"{"id":"1.20.1"}"#).unwrap();
        fs::write(paths.version_dir("1.20.1").join("instance.json"), r#"{"name":"Source"}"#).unwrap();

        // 首次复制:id 由名字 slug 化。
        let id1 = copy_instance_named(&paths, "1.20.1", "我的副本").unwrap();
        assert_eq!(id1, "我的副本");
        let cfg1 = Instance::new(id1.clone(), paths.root().to_path_buf()).load_config().unwrap();
        assert_eq!(cfg1.name.as_deref(), Some("我的副本"), "新实例名应改写为给定名");

        // 再复制同名:id 自动加后缀避免目录冲突,name 仍为给定名。
        let id2 = copy_instance_named(&paths, "1.20.1", "我的副本").unwrap();
        assert_eq!(id2, "我的副本-2");
        let cfg2 = Instance::new(id2, paths.root().to_path_buf()).load_config().unwrap();
        assert_eq!(cfg2.name.as_deref(), Some("我的副本"));
    }

    #[test]
    fn copy_instance_rejects_existing_target() {
        let root = TempRoot::new("copy-exists");
        let paths = root.paths();

        fs::create_dir_all(paths.version_dir("a")).unwrap();
        fs::write(paths.version_json("a"), r#"{"id":"a"}"#).unwrap();
        // 目标已存在。
        fs::create_dir_all(paths.version_dir("b")).unwrap();

        let err = copy_instance(&paths, "a", "b").unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "目标已存在应返回 Other 错误");
    }

    #[test]
    fn copy_instance_rejects_missing_source() {
        let root = TempRoot::new("copy-missing");
        let paths = root.paths();
        let err = copy_instance(&paths, "nope", "dst").unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "源不存在应返回 Other 错误");
    }

    #[test]
    fn copy_instance_without_jar_succeeds() {
        // 没有客户端 jar(如纯继承的 loader profile)时,复制仍应成功。
        let root = TempRoot::new("copy-nojar");
        let paths = root.paths();
        fs::create_dir_all(paths.version_dir("src")).unwrap();
        fs::write(paths.version_json("src"), r#"{"id":"src","inheritsFrom":"1.20.1"}"#).unwrap();

        copy_instance(&paths, "src", "dst").unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(paths.version_json("dst")).unwrap()).unwrap();
        assert_eq!(parsed["id"], "dst");
        assert_eq!(parsed["inheritsFrom"], "1.20.1");
        assert!(!paths.version_jar("dst").exists(), "源无 jar 时目标也不应有 jar");
    }

    // ---- add_loader: re-id + relink (network-free parts) ----

    #[test]
    fn add_loader_validation_rejects_vanilla() {
        // 给实例加「原版」无意义,应被拒绝。整段 add_loader 走异步 + 真实下载器,
        // 这里只验证早返回的校验分支(不触发任何网络)。
        let root = TempRoot::new("add-loader-vanilla");
        let paths = root.paths();
        fs::create_dir_all(paths.version_dir("1.20.1")).unwrap();
        fs::write(paths.version_json("1.20.1"), r#"{"id":"1.20.1"}"#).unwrap();

        let dl = crate::download::Downloader::new(1).unwrap();
        let err = futures::executor::block_on(add_loader(
            &dl,
            &paths,
            "1.20.1",
            "1.20.1",
            (LoaderKind::Vanilla, String::new()),
            None,
        ))
        .unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "vanilla 应被拒绝");
    }

    #[test]
    fn add_loader_unique_id_for_degenerate() {
        // 退化情形的新 id 命名:{instance_id}-{loader}(小写),冲突加序号。
        let root = TempRoot::new("add-loader-uid");
        let paths = root.paths();
        assert_eq!(unique_loader_instance_id(&paths, "1.20.1", LoaderKind::Fabric), "1.20.1-fabric");

        // 该名已被占用时加序号。
        fs::create_dir_all(paths.version_dir("1.20.1-fabric")).unwrap();
        assert_eq!(unique_loader_instance_id(&paths, "1.20.1", LoaderKind::Fabric), "1.20.1-fabric-2");

        // NeoForge 小写化。
        assert_eq!(
            unique_loader_instance_id(&paths, "1.21", LoaderKind::NeoForge),
            "1.21-neoforge"
        );
    }

    #[test]
    fn relink_instance_stub_overwrites_with_inherits() {
        // (i) 常规情形的核心动作:把存根 json 重写为 {id, inheritsFrom: core_id}。
        let root = TempRoot::new("relink");
        let paths = root.paths();
        // 一个已带 inheritsFrom 的薄实例存根(继承原版)。
        fs::create_dir_all(paths.version_dir("my-pack")).unwrap();
        fs::write(
            paths.version_json("my-pack"),
            r#"{"id":"my-pack","inheritsFrom":"1.20.1"}"#,
        )
        .unwrap();

        // 重指向 loader 核心(模拟 install_core 返回的 core_id)。
        relink_instance_stub(&paths, "my-pack", "fabric-loader-0.15.7-1.20.1").unwrap();

        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(paths.version_json("my-pack")).unwrap()).unwrap();
        assert_eq!(v["id"], "my-pack", "实例 id 不变");
        assert_eq!(
            v["inheritsFrom"], "fabric-loader-0.15.7-1.20.1",
            "应重指向 loader 核心"
        );
        // 存根只保留这两个键(薄存根语义)。
        assert_eq!(v.as_object().unwrap().len(), 2);
    }

    #[test]
    fn rename_instance_dir_moves_and_rewrites_id() {
        // (ii) 退化情形的核心动作:把裸原版目录整体改名到新 id,内部 json/jar 一并改名,
        // 改名后原 id(== mc_version)的目录消失,可供 install_core 重建原版。
        let root = TempRoot::new("reid");
        let paths = root.paths();

        // 造一个「实例目录就是裸原版」的退化实例:id "1.20.1",带 jar、mod、icon、instance.json。
        let old_dir = paths.version_dir("1.20.1");
        fs::create_dir_all(&old_dir).unwrap();
        fs::write(
            paths.version_json("1.20.1"),
            r#"{"id":"1.20.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )
        .unwrap();
        fs::write(paths.version_jar("1.20.1"), b"FAKEJAR").unwrap();
        fs::create_dir_all(old_dir.join("mods")).unwrap();
        fs::write(old_dir.join("mods/sodium.jar"), b"MODBYTES").unwrap();
        fs::write(old_dir.join("icon.png"), b"\x89PNGicon").unwrap();
        fs::write(old_dir.join("instance.json"), r#"{"name":"My World","memory_mb":4096}"#).unwrap();

        rename_instance_dir(&paths, "1.20.1", "1.20.1-fabric").unwrap();

        // 原 id 目录已不存在 → mc_version "1.20.1" 这个名字腾出,可被 install_core 重建。
        assert!(!old_dir.exists(), "原裸原版目录应已改名消失");

        // 新目录:json 改名 + 内部 id 改写,其余字段保留。
        let new_json = paths.version_json("1.20.1-fabric");
        assert!(new_json.is_file(), "应生成 1.20.1-fabric.json");
        let new_dir = paths.version_dir("1.20.1-fabric");
        assert!(!new_dir.join("1.20.1.json").exists(), "旧名 json 应被删除");
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&new_json).unwrap()).unwrap();
        assert_eq!(v["id"], "1.20.1-fabric");
        assert_eq!(v["mainClass"], "net.minecraft.client.main.Main");
        assert_eq!(v["type"], "release");

        // jar 随 id 改名。
        assert!(paths.version_jar("1.20.1-fabric").is_file(), "jar 应改名为 1.20.1-fabric.jar");
        assert!(!new_dir.join("1.20.1.jar").exists());

        // 游戏数据 / icon / instance.json 随目录迁移。
        assert_eq!(fs::read(new_dir.join("mods/sodium.jar")).unwrap(), b"MODBYTES");
        assert_eq!(fs::read(new_dir.join("icon.png")).unwrap(), b"\x89PNGicon");
        assert!(new_dir.join("instance.json").is_file());

        // 模拟 add_loader 退化分支后续:install_core 重建 "1.20.1" 原版 + 把新实例重指向 loader 核心。
        fs::create_dir_all(&old_dir).unwrap();
        fs::write(paths.version_json("1.20.1"), r#"{"id":"1.20.1","type":"release"}"#).unwrap();
        relink_instance_stub(&paths, "1.20.1-fabric", "fabric-loader-0.15.7-1.20.1").unwrap();

        // 原 mc_version 原版重新可解析。
        assert!(paths.version_json("1.20.1").is_file(), "重建后的原版应可解析");
        // 新实例已重指向 loader 核心,id 为新 id。
        let relinked: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(new_json).unwrap()).unwrap();
        assert_eq!(relinked["id"], "1.20.1-fabric");
        assert_eq!(relinked["inheritsFrom"], "fabric-loader-0.15.7-1.20.1");
    }

    #[test]
    fn rename_instance_dir_rejects_existing_target() {
        let root = TempRoot::new("reid-exists");
        let paths = root.paths();
        fs::create_dir_all(paths.version_dir("1.20.1")).unwrap();
        fs::write(paths.version_json("1.20.1"), r#"{"id":"1.20.1"}"#).unwrap();
        fs::create_dir_all(paths.version_dir("1.20.1-fabric")).unwrap();

        let err = rename_instance_dir(&paths, "1.20.1", "1.20.1-fabric").unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "目标已存在应返回 Other 错误");
    }

    // ---- rewrite_version_id ----

    #[test]
    fn rewrite_version_id_preserves_other_fields() {
        let raw = r#"{
            "id": "old-id",
            "inheritsFrom": "1.20.1",
            "type": "release",
            "libraries": [{"name": "a:b:1"}],
            "arguments": {"game": ["--foo"]}
        }"#;
        let out = rewrite_version_id(raw, "new-id").unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["id"], "new-id");
        assert_eq!(v["inheritsFrom"], "1.20.1");
        assert_eq!(v["type"], "release");
        assert_eq!(v["libraries"][0]["name"], "a:b:1");
        assert_eq!(v["arguments"]["game"][0], "--foo");
    }

    #[test]
    fn rewrite_version_id_rejects_non_object() {
        let err = rewrite_version_id("[1,2,3]", "x").unwrap_err();
        assert!(matches!(err, CoreError::Other(_)));
    }

    // ---- delete_instance ----

    #[test]
    fn delete_instance_removes_dir_and_is_idempotent() {
        let root = TempRoot::new("delete");
        let paths = root.paths();
        let dir = paths.version_dir("doomed");
        fs::create_dir_all(&dir).unwrap();
        fs::write(paths.version_json("doomed"), r#"{"id":"doomed"}"#).unwrap();

        delete_instance(&paths, "doomed").unwrap();
        assert!(!dir.exists(), "删除后目录应不在原位(回收站或硬删均可)");

        // 重复删除不存在的实例应幂等成功。
        delete_instance(&paths, "doomed").unwrap();
    }

    // ---- mrpack 索引解析 ----

    #[test]
    fn parse_mrpack_index_inline() {
        let sample = r#"{
            "formatVersion": 1,
            "game": "minecraft",
            "name": "My Modpack",
            "versionId": "1.0.0",
            "dependencies": {
                "minecraft": "1.20.1",
                "fabric-loader": "0.15.7"
            },
            "files": [
                {
                    "path": "mods/sodium.jar",
                    "downloads": ["https://cdn.modrinth.com/data/x/sodium.jar"],
                    "hashes": { "sha1": "deadbeef", "sha512": "long" },
                    "fileSize": 123456,
                    "env": { "client": "required", "server": "optional" }
                },
                {
                    "path": "mods/server-only.jar",
                    "downloads": ["https://example.com/server-only.jar"],
                    "hashes": { "sha1": "cafe" },
                    "env": { "client": "unsupported", "server": "required" }
                },
                {
                    "path": "config/only-sha512.toml",
                    "downloads": ["https://example.com/cfg.toml"],
                    "hashes": { "sha512": "onlybig" }
                }
            ]
        }"#;

        let index: MrpackIndex = serde_json::from_str(sample).unwrap();
        assert_eq!(index.format_version, 1);
        assert_eq!(index.name, "My Modpack");
        assert_eq!(index.dependencies.minecraft.as_deref(), Some("1.20.1"));
        assert_eq!(index.dependencies.fabric_loader.as_deref(), Some("0.15.7"));
        assert_eq!(index.files.len(), 3);

        // 文件 0:client required → 受支持;sha1 / size / downloads 正确。
        let f0 = &index.files[0];
        assert_eq!(f0.path, "mods/sodium.jar");
        assert!(f0.client_supported());
        assert_eq!(f0.downloads.first().map(String::as_str), Some("https://cdn.modrinth.com/data/x/sodium.jar"));
        assert_eq!(f0.hashes.sha1.as_deref(), Some("deadbeef"));
        assert_eq!(f0.file_size, Some(123456));

        // 文件 1:client unsupported → 应被跳过。
        assert!(!index.files[1].client_supported());

        // 文件 2:无 env → 受支持(缺省)、sha1 None。
        let f2 = &index.files[2];
        assert!(f2.client_supported());
        assert!(f2.hashes.sha1.is_none());
        assert_eq!(f2.file_size, None);
    }

    #[test]
    fn parse_mrpack_index_minimal() {
        // 仅含必需字段(game/name/dependencies.minecraft)的最小索引也应解析成功。
        let sample =
            r#"{"formatVersion":1,"game":"minecraft","name":"Mini","dependencies":{"minecraft":"1.21"}}"#;
        let index: MrpackIndex = serde_json::from_str(sample).unwrap();
        assert_eq!(index.dependencies.minecraft.as_deref(), Some("1.21"));
        assert!(index.files.is_empty());
        assert_eq!(index.name, "Mini");
    }

    // ---- export / import overrides 往返 ----

    #[test]
    fn export_then_reimport_overrides_roundtrip() {
        let root = TempRoot::new("export");
        // 造实例:mods + config,带嵌套目录。
        let inst = Instance::new("1.20.1", root.path.clone());
        let game_dir = inst.game_dir();
        fs::create_dir_all(game_dir.join("mods")).unwrap();
        fs::write(game_dir.join("mods/cool.jar"), b"COOLMOD").unwrap();
        fs::create_dir_all(game_dir.join("config/sub")).unwrap();
        fs::write(game_dir.join("config/sub/opts.toml"), b"key=1").unwrap();
        // 给实例起个名,验证写进索引。
        let mut cfg = inst.load_config().unwrap();
        cfg.name = Some("Exported Pack".to_string());
        inst.save_config(&cfg).unwrap();

        // 导出。
        let dest = root.path.join("out.mrpack");
        export_mrpack(&inst, "1.20.1", &dest).unwrap();
        assert!(dest.is_file());

        // 打开导出的 zip,校验索引与 overrides 内容。
        let f = fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();

        // 索引存在且内容正确。
        let index: MrpackIndex = {
            let mut e = archive.by_name(MRPACK_INDEX_ENTRY).unwrap();
            let mut s = String::new();
            e.read_to_string(&mut s).unwrap();
            serde_json::from_str(&s).unwrap()
        };
        assert_eq!(index.format_version, 1);
        assert_eq!(index.name, "Exported Pack");
        assert_eq!(index.dependencies.minecraft.as_deref(), Some("1.20.1"));
        assert!(index.files.is_empty(), "本地导出 files 应为空,全部走 overrides");

        // overrides 条目存在。
        let mut cool = archive.by_name("overrides/mods/cool.jar").unwrap();
        let mut buf = Vec::new();
        cool.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"COOLMOD");
        drop(cool);
        let mut opts = archive.by_name("overrides/config/sub/opts.toml").unwrap();
        let mut buf2 = Vec::new();
        opts.read_to_end(&mut buf2).unwrap();
        assert_eq!(buf2, b"key=1");
        drop(opts);

        // 再用导入侧的归档解压器(ZipArchiveIndex::extract_prefix)解到一个新 game_dir,
        // 验证导出的 overrides 能被导入管线正确还原(往返闭环;override 铺设细节的
        // 单测在 modpack::import::archive::tests)。
        drop(archive);
        let target = root.path.join("reimport-game-dir");
        fs::create_dir_all(&target).unwrap();
        let mut idx = crate::modpack::import::archive::ZipArchiveIndex::open(&dest).unwrap();
        idx.extract_prefix(OVERRIDES_PREFIX, &target).unwrap();
        assert_eq!(fs::read(target.join("mods/cool.jar")).unwrap(), b"COOLMOD");
        assert_eq!(fs::read(target.join("config/sub/opts.toml")).unwrap(), b"key=1");
    }

    #[test]
    fn export_writes_index_and_overrides_prefix() {
        // 导出产物的结构契约:索引为 formatVersion=1 且 files 为空,本地数据落在
        // overrides/ 前缀下(client-overrides 覆盖语义的单测在 import::archive::tests)。
        let root = TempRoot::new("export-struct");
        let inst = Instance::new("1.20.1", root.path.clone());
        let game_dir = inst.game_dir();
        fs::create_dir_all(game_dir.join("config")).unwrap();
        fs::write(game_dir.join("config/shared.txt"), b"GENERIC").unwrap();

        let dest = root.path.join("ov.mrpack");
        export_mrpack(&inst, "1.20.1", &dest).unwrap();

        let f = fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let mut shared = archive.by_name("overrides/config/shared.txt").unwrap();
        let mut buf = String::new();
        shared.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "GENERIC");
    }

    #[test]
    fn import_archive_extract_blocks_zip_slip() {
        // 导入侧解压同样拒绝 zip-slip(与导出无关的安全闸,经 ZipArchiveIndex)。
        let root = TempRoot::new("slip");
        let dest = root.path.join("evil.mrpack");
        {
            let out = fs::File::create(&dest).unwrap();
            let mut zw = zip::ZipWriter::new(out);
            let opt = zip::write::SimpleFileOptions::default();
            // 一个试图越权写到父目录的条目。
            zw.start_file("overrides/../../escaped.txt", opt).unwrap();
            zw.write_all(b"PWNED").unwrap();
            zw.finish().unwrap();
        }
        let target = root.path.join("game-dir");
        fs::create_dir_all(&target).unwrap();
        let mut idx = crate::modpack::import::archive::ZipArchiveIndex::open(&dest).unwrap();
        let err = idx.extract_prefix(OVERRIDES_PREFIX, &target).unwrap_err();
        assert!(matches!(err, CoreError::Other(_)), "zip-slip 应被拒绝");
    }
