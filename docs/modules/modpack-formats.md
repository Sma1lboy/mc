# 模块 · 整合包格式参考(精确 Schema → Rust 结构)

> 每种整合包/实例格式的**字段级**精确结构,可直接落成 `serde` 类型。配 [modpack-import.md](./modpack-import.md)
> 的可插拔 importer 使用:每种格式一个 `plan()` 解析这里的结构 → 统一 `ImportPlan`。
> 字段已对照 `ref/` 源码核实;**易错点**(serde rename、可空、默认值反转、内容判别)单独标注。

## 0. 探测标记与优先级

| 序 | 格式 | 标记文件 | 判别要点 |
|----|------|----------|----------|
| 1 | MCBBS(国内) | `mcbbs.packmeta` | 在 manifest.json 前 |
| 2 | MultiMC/Prism | basename `mmc-pack.json` / `instance.cfg` | 目录即包,可嵌套一层 |
| 3 | Modrinth | `modrinth.index.json` | 根级 |
| 4 | CurseForge | `manifest.json`(`manifestType=="minecraftModpack"` 且**无** `addons`) | manifest.json 最后判 |
| 4b | MCBBS(变体) | `manifest.json`(**有** `addons`/`launchInfo`) | 与 CF 同名,**读内容**区分 |
| 5 | packwiz | `pack.toml` | TOML,非 zip,常为 git 仓库/HTTP |
| 6 | Technic | `bin/modpack.jar` / `bin/version.json` | 解压到 `minecraft/` |
| 7 | ATLauncher | 远程 `Configs.json`(无本地标记) | 用户从平台选包 |

> `.rar` 不支持:给出「请重新压缩为 zip」的清晰错误(PCL2 行为),不要误判。

## 1. Modrinth `.mrpack`

布局:zip,根有唯一 `modrinth.index.json`(`formatVersion==1`、`game=="minecraft"`)+ 可选 `overrides/`(客户端+服务端都铺)、`client-overrides/`(仅客户端,盖在 overrides 上)。`files[]` 里的文件**不在包内**,要下载。

```rust
#[derive(Serialize, Deserialize)]
pub struct MrpackIndex {
    #[serde(rename = "formatVersion")] pub format_version: u32, // 必须 1
    pub game: String,                                           // 必须 "minecraft"
    #[serde(rename = "versionId", default)] pub version_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub summary: Option<String>,
    pub dependencies: MrpackDependencies,
    #[serde(default)] pub files: Vec<MrpackFile>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]  // 复刻 Prism「Unknown dependency type」抛错
pub struct MrpackDependencies {
    #[serde(default)] pub minecraft: Option<String>,
    #[serde(rename = "fabric-loader", default)] pub fabric_loader: Option<String>,
    #[serde(rename = "quilt-loader", default)] pub quilt_loader: Option<String>,
    #[serde(default)] pub forge: Option<String>,
    #[serde(default)] pub neoforge: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct MrpackFile {
    pub path: String,                       // 相对游戏目录;反斜杠归一为 '/';必过 safe_join
    pub hashes: MrpackHashes,
    #[serde(default)] pub env: Option<MrpackEnv>,
    pub downloads: Vec<String>,             // 有序候选 URL,非空;hosts 受白名单约束
    #[serde(rename = "fileSize", default)] pub file_size: Option<u64>,
}

#[derive(Serialize, Deserialize)]
pub struct MrpackHashes { pub sha512: String, #[serde(default)] pub sha1: Option<String> }

#[derive(Serialize, Deserialize)]
pub struct MrpackEnv { pub client: EnvSupport, pub server: EnvSupport }

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvSupport { Required, Optional, Unsupported }
```

要点:
- **Prism 强制 `sha512`**(用作校验 + 更新去重键);`sha1` 可缺。→ 下载器需 sha512(见 [download.md](./download.md) §5)。
- `env.client == Unsupported` → 整个文件跳过;`== Optional` → 降级为可选(默认落 `.disabled`)。导入只看 client。
- **下载 host 白名单**:`cdn.modrinth.com / github.com / raw.githubusercontent.com / gitlab.com`。Prism 只在**导出**时校验它(`MODRINTH_MRPACK_HOSTS`);**导入不校验,只靠 sha512 + 路径穿越防护**。建议我们**导入也校验**作纵深防御。
- URL 用宽松模式解析(Modrinth 偶发未转义空格)。

## 2. packwiz(TOML,两级)

不是 zip,是一棵 TOML 文件(常为 git 仓库 / HTTP)。Prism 只解析**单 mod** `<slug>.pw.toml`;`pack.toml`/`index.toml` 按 packwiz 官方 spec 补全(标注为非 Prism 来源)。

```
pack.toml          根清单:name, versions{minecraft, fabric/forge/...}, index{file, hash-format, hash}
index.toml         文件清单:hash-format + files[]{file, hash, metafile(bool), preserve(bool)}
mods/<slug>.pw.toml  metafile=true 的条目指向它:name, filename, side, download{mode, url, hash-format, hash}, update{modrinth|curseforge}
```

`metafile=false` 的条目是就地哈希的真实文件(config 等);Prism 还往 `.pw.toml` 注 `x-prismlauncher-*` 扩展键(读时必须可选)。

## 3. CurseForge(manifest.json + Flame API)

zip 根:`manifest.json`(`manifestType=="minecraftModpack"`、`manifestVersion==1`)+ `overrides/`(名由 `manifest.overrides` 定,默认 `overrides`)+ 可选 `modlist.html`(忽略)。`files[]` **只给 projectID/fileID**,要经 Flame API(`api.curseforge.com`,需 `x-api-key`)解析为真实 URL。

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameManifest {
    pub manifest_type: String,              // 必须 "minecraftModpack"
    pub manifest_version: i32,              // 必须 1
    pub minecraft: FlameMinecraft,
    #[serde(default = "default_pack_name")] pub name: String,     // 默认 "Unnamed"
    #[serde(default)] pub version: String,
    #[serde(default = "default_author")] pub author: String,      // 默认 "Anonymous"
    #[serde(default)] pub files: Vec<FlameManifestFile>,
    #[serde(default = "default_overrides")] pub overrides: String, // 默认 "overrides"
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameMinecraft {
    pub version: String,                    // MC 版本在这里,不在 loader id 里
    #[serde(default)] pub libraries: String,// 1.2.5 FTB 遗留,忽略
    #[serde(default)] pub mod_loaders: Vec<FlameModLoader>,
    #[serde(default)] pub recommended_ram: i32, // JSON 键是 camelCase "recommendedRam"!
}

#[derive(Serialize, Deserialize)]
pub struct FlameModLoader {
    pub id: String,                         // "forge-47.2.0"/"fabric-0.15.7"/"neoforge-..."/"quilt-..." → split_once('-')
    #[serde(default)] pub primary: bool,    // 默认 false;装 primary 的那个(无则取第一个)
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameManifestFile {
    #[serde(rename = "projectID")] pub project_id: i64,
    #[serde(rename = "fileID")] pub file_id: i64,
    #[serde(default = "default_true")] pub required: bool, // 默认 TRUE,是 'optional' 的反义,极易弄反
}
```

Flame API 解析(`resolve()` 用):`POST /v1/mods/files {fileIds:[...]}` → `FlameApiFile`;`POST /v1/mods {modIds:[...]}` 补名字/slug;`POST /v1/fingerprints {fingerprints:[...]}` murmur2 反查本地 jar。

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameApiFile {
    pub id: i64, pub mod_id: i64,
    #[serde(default)] pub display_name: String,
    pub file_name: String,                  // 写盘前必须 RemoveInvalidPathChars + 拒绝分隔符
    pub release_type: i32,                   // 1=Release 2=Beta 3=Alpha
    #[serde(default)] pub download_url: Option<String>, // **可空!** null/空 = BLOCKED 不可再分发
    #[serde(default)] pub file_length: Option<u64>,
    #[serde(default)] pub file_fingerprint: Option<u64>, // murmur2(滤空白字节, seed 1)
    #[serde(default)] pub hashes: Vec<FlameApiFileHash>,  // algo: 1=sha1 2=md5(整数枚举!)
    #[serde(default)] pub game_versions: Vec<String>,     // 混杂:MC 版本(含'.') + loader 名 + Client/Server
    #[serde(default)] pub dependencies: Vec<FlameApiFileDependency>, // relationType 1=Embedded 2=Optional 3=Required 4=Tool 5=Incompatible 6=Include
}
```

要点:
- **`downloadUrl` 可空 = BLOCKED**(作者禁第三方分发):必须走手动下载流(给 `websiteUrl/download/{fileId}`),**绝不**猜 URL。这是 CF 导入的核心边界。
- **murmur2 不是标准 murmur2**:seed=1,**先滤掉字节 9/10/13/32**(tab/LF/CR/空格)再算,4MiB 分块。算错则零文件匹配。
- `hashes[].algo` 是**整数**(1=sha1,2=md5),不是字符串;CF 只保 sha1。
- `gameVersions[]` 是**扁平异构数组**(MC 版本 + loader 名 + Client/Server),必须客户端切分。
- `files[].required` 默认 **true**,是 optional 的反义。

## 4. MultiMC / Prism 原生(目录即包)

不是单文件,是实例**目录**(导出 = 该目录打 zip)。两文件驱动:`instance.cfg`(无 section 的 INI,Qt QSettings)+ `mmc-pack.json`(组件图)+ `patches/<uid>.json`(每组件覆盖)+ 游戏目录 `minecraft/`(回退 `.minecraft/`)。**内容是预装好的 loose 文件,无远程 files[]。**

```rust
#[derive(Serialize, Deserialize)]
pub struct MmcPack { #[serde(rename = "formatVersion")] pub format_version: u32, pub components: Vec<MmcComponent> } // formatVersion 必须 1;顺序即合并序

#[derive(Serialize, Deserialize)]
pub struct MmcComponent {
    pub uid: String,                        // net.minecraft / net.fabricmc.fabric-loader / net.minecraftforge / net.neoforged / org.quiltmc.quilt-loader / org.lwjgl3 / custom.jarmod.<uuid>
    #[serde(default, skip_serializing_if = "Option::is_none")] pub version: Option<String>,
    #[serde(default, skip_serializing_if = "is_false", rename = "dependencyOnly")] pub dependency_only: bool,
    #[serde(default, skip_serializing_if = "is_false")] pub important: bool,  // net.minecraft / 所选 loader → 不可删
    #[serde(default, skip_serializing_if = "is_false")] pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "cachedVersion")] pub cached_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "cachedName")] pub cached_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "cachedRequires")] pub cached_requires: Vec<MmcRequire>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "cachedConflicts")] pub cached_conflicts: Vec<MmcRequire>,
    // bug-for-bug:Prism 写 "cachedVolatile" 但读 "volatile" → 输入接受两者,输出写 cachedVolatile
    #[serde(default, skip_serializing_if = "is_false", rename = "cachedVolatile", alias = "volatile")] pub cached_volatile: bool,
}

#[derive(Serialize, Deserialize)]
pub struct MmcRequire {
    pub uid: String,
    #[serde(default, rename = "equals")] pub equals_version: Option<String>, // JSON 键是 "equals"
    #[serde(default)] pub suggests: Option<String>,
}
```

`instance.cfg`(无 section INI,**不是 JSON**,用按行解析器):关键键 `name / InstanceType=OneSix / iconKey / notes / ConfigVersion=1.3`;`Override*` 闸门(`OverrideMemory`→`MinMemAlloc/MaxMemAlloc`、`OverrideJavaArgs`→`JvmArgs`、`OverrideWindow`→窗口尺寸…只在闸=true 时生效,否则继承全局)、`JoinServerOnLaunch*`、统计、以及**跨格式溯源** `ManagedPack/ManagedPackType/ManagedPackID/ManagedPackVersionID/...`(这实例来自 modrinth/flame/…)。保留 `raw` 全键以无损往返;映射到统一模型时 `Override*` 闸用 `Option<T>`(None=继承)。

加载器 uid 表(`KNOWN_MODLOADERS`):`net.neoforged`→NeoForge、`net.minecraftforge`→Forge、`net.fabricmc.fabric-loader`→Fabric、`org.quiltmc.quilt-loader`→Quilt、`com.mumfrey.liteloader`→LiteLoader;`org.lwjgl*`/`net.minecraft` **不是** loader。Quilt 蕴含 Fabric 支持;1.20.1 上 NeoForge 蕴含 Forge。

## 5. MCBBS(国内)/ HMCL-lineage

zip,根 `mcbbs.packmeta`(或带 `addons` 的 `manifest.json`)+ `overrides/`。mod 不在包内,经 CurseForge/Modrinth 拉。

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McbbsPackMeta {
    #[serde(default)] pub manifest_type: Option<String>,   // "minecraftModpack"(野包常缺,别强求)
    #[serde(default)] pub manifest_version: Option<u32>,
    #[serde(default)] pub name: Option<String>,
    #[serde(default)] pub version: Option<String>,
    #[serde(default)] pub author: Option<String>,
    #[serde(default)] pub url: Option<String>,
    #[serde(default)] pub file_api: Option<String>,        // 更新/镜像基址
    #[serde(default)] pub force_override: Option<bool>,
    #[serde(default)] pub files: Vec<McbbsFile>,            // CurseForge-shaped {projectID,fileID,type,...}
    #[serde(default)] pub addons: Vec<McbbsAddon>,          // ← 加载器谱:扁平 id→version
    #[serde(default)] pub launch_info: Option<McbbsLaunchInfo>,
    #[serde(default)] pub settings: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
pub struct McbbsAddon { pub id: String, pub version: String } // id: "game"(必需) | forge | neoforge | fabric | quilt | optifine | liteloader

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McbbsLaunchInfo {
    #[serde(default)] pub min_memory: Option<u32>,
    #[serde(default)] pub supported_java_versions: Vec<u32>,
    #[serde(default)] pub launch_argument: Vec<String>,    // → 实例游戏参数
    #[serde(default)] pub java_argument: Vec<String>,      // → 实例 JVM 参数
    #[serde(default)] pub pre_launch_command: Option<String>,
    #[serde(default)] pub post_exit_command: Option<String>,
}
```

要点:**靠 `addons` 存在区分于 CurseForge**;`addons[id=="game"]` 是 MC 版本(必需);PCL2 硬拒 quilt 但 **mc-core 可支持**(有 `loader/quilt`);`launchInfo.{launch_argument,java_argument}` → 实例参数;其余 settings/forceOverride 首装可忽略。

## 6. ATLauncher / Technic(远程,长尾)

**非本地 zip**,走 [modpack-import.md](./modpack-import.md) §6 的 RemotePackProvider:

- **ATLauncher**:`safeName`(去非字母数字)→ `GET .../Configs.json` 得 `AtlPackVersion{version, minecraft, loader{type,metadata}, mods[], configs{sha1}, mainClass, extraArguments, keeps/deletes}`。mod `download` ∈ `server|direct|browser`(browser=blocked 手动);`type` 路由目标目录;`extractTo/decompType` 解压;md5 校验;`client==false` 跳过。`Configs.zip` 是 overrides 包。
- **Technic**:(a) 单 zip——整包解压进 `.minecraft`,无 manifest,MC 版本来自 Platform API,loader 靠解压后嗅探;(b) Solder——`GET {solder}/modpack/{pack}/{version}` 得 `{minecraft, mods:[{name,version,md5,url}]}`,每个 `url` 是小 zip **按序**叠加(后盖前),loader 嗅探。

> 统一映射:三者都收敛到同一 `ImportPlan`。**ATL/Technic 的文件是 zip(解压)而非 drop-in jar**,且只给 md5 → 统一模型需要:`VersionFile` 加可选 md5(或哈希枚举)+ 每文件 `kind: Drop | ExtractZip`。

## 7. 跨格式安全清单(所有 importer 共用)

- **Zip-slip / 路径穿越**:每个 overrides 拷贝与每个解压条目都 `safe_join`(规整后须在 game_dir 内,拒 `..`/绝对/盘符/符号链接逃逸)。`manifest.overrides`、`patches/<uid>.json` 的 uid、CF `fileName`、ATL `extractFolder`(`%s%` 分隔)全是攻击面。
- **下载 host 白名单**:CF 仅 `*.forgecdn.net`;Modrinth 仅四白名单 host;ATL/Technic 的 `url` 任意 → **强制** md5/sha1 校验(两格式都给哈希)。
- **blocked / 不可分发**:CF `downloadUrl==null`、ATL `download=="browser"` → 手动流,绝不伪造 URL。
- **manifest 闸**:`manifestType`/`manifestVersion` 不符直接拒,不做「尽力解析」(会错配 id)。
- **API key**:CurseForge 需 `x-api-key`,按项目 secrets 约定放 env、勿入库、勿打日志。
- **资源耗尽**:校验声明大小、限并发与总量、流式解压防 zip 炸弹。
- **导入的 JavaPath/启动命令不可自动信任**(MultiMC instance.cfg):丢弃或显式让用户确认(会执行任意二进制)。

> 这些结构是 importer `plan()` 的输入边界。统一产物见 [modpack-import.md](./modpack-import.md) 的 `ImportPlan`。
