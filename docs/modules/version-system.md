# 模块 · 版本/组件系统

> 启动器最核心也最绕的部分:把一堆 json 算成一份可启动的配置。

## 1. 两种版本模型

| 模型 | 代表 | 原理 |
|------|------|------|
| **继承模型** | PCL 系、官方启动器、HMCL | loader 版本 json 里写 `inheritsFrom: "1.20.1"`,递归把父版本字段并进来 |
| **组件模型** | PrismLauncher / MultiMC | 实例维护一个**有序组件列表**(`mmc-pack.json`),每个组件是独立的 uid+version,启动时按序 `applyTo` 合并 |

两者表达力等价。**组件模型更清晰、更易扩展**(能分别升降级 loader、显示依赖关系),推荐自研采用;继承模型实现更简单,且与官方/HMCL 互通性好。

## 2. 版本 json 结构(Mojang 格式)

```jsonc
{
  "id": "1.20.1",
  "mainClass": "net.minecraft.client.main.Main",
  "assetIndex": { "id": "5", "url": "...", "sha1": "...", "totalSize": ... },
  "assets": "5",
  "downloads": {
    "client": { "url": "...", "sha1": "...", "size": ... },
    "server": { ... }
  },
  "javaVersion": { "component": "java-runtime-gamma", "majorVersion": 17 },
  "libraries": [
    {
      "name": "org.lwjgl:lwjgl:3.3.1",            // Gradle 坐标
      "downloads": {
        "artifact": { "path": "...", "url": "...", "sha1": "...", "size": ... },
        "classifiers": { "natives-windows": { ... } }   // 老式 native
      },
      "rules": [ { "action": "allow", "os": { "name": "windows" } } ],
      "natives": { "windows": "natives-windows" }        // 老式 native 映射
    }
  ],
  "arguments": {                                    // 1.13+ 新格式
    "game": [ "--username", "${auth_player_name}", { "rules": [...], "value": [...] } ],
    "jvm":  [ "-Djava.library.path=${natives_directory}", "-cp", "${classpath}" ]
  },
  "minecraftArguments": "--username ${auth_player_name} ...",  // 1.12- 老格式(二选一)
  "type": "release"
}
```

**关键点**:
- `arguments`(数组,1.13+)与 `minecraftArguments`(字符串,1.12-)互斥,都要支持。
- 数组元素可能是纯字符串,也可能是 `{rules, value}` 对象(条件参数)。
- `javaVersion.majorVersion` 决定需要哪个 Java。

## 3. Gradle 坐标与库路径

库名是 Maven/Gradle 坐标:`group:artifact:version[:classifier][@ext]`

转本地路径规则:`group(点→斜杠)/artifact/version/artifact-version[-classifier].jar`

例:`org.lwjgl:lwjgl:3.3.1:natives-windows`
→ `org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar`

> Prism 的 `GradleSpecifier.h` 是这块的精炼参考。

## 4. <a name="rule"></a>Rule 规则过滤(OS/架构/特性)

库和参数都可能带 `rules` 数组,决定"是否启用"。求值规则:

1. 默认 disallow(若有任何 allow 规则);无 rules 则默认启用。
2. 顺序应用每条 rule,匹配当前环境(OS name / arch / version 正则、feature 开关)就采用其 action。
3. 最后一条匹配的 action 决定结果。

```jsonc
"rules": [
  { "action": "allow" },
  { "action": "disallow", "os": { "name": "osx" } }   // 除 macOS 外都启用
]
```

feature 规则用于游戏参数(如 `is_demo_user`、`has_custom_resolution`),由启动器按用户设置传入。

> Prism `Rule.h` + `RuntimeContext`(抽象 OS/arch/java)是干净的实现参考。

## 5. natives 处理

两种格式:
- **老式**(1.18-):库带 `natives` 映射 + `classifiers`,native jar 是带 classifier 的同名库。
- **新式**(1.19+):native 直接作为独立库条目带 `natives-xxx` classifier,用 rules 控制平台。

流程:筛出当前平台的 native jar → 解压其中的 `.dll/.so/.dylib`(排除 `META-INF`)到临时 `natives/` 目录 → 启动时 `-Djava.library.path` 指过去。

## 6. 组件合并(applyTo)

以组件模型为例,启动时:

```
LaunchProfile  = 空
for each 启用的组件 (按列表顺序, 通常 vanilla 在前, loader 在后):
    component.applyTo(LaunchProfile):
        - libraries: 追加(loader 可能要求覆盖同名库的版本)
        - mainClass: 覆盖(loader 改成自己的引导类)
        - arguments: 追加(jvm + game)
        - traits:    合并(如 "legacyLaunch")
        - assetIndex / mainJar: 一般 vanilla 提供,loader 不动
```

冲突与依赖:组件可声明 `requires`(依赖某 uid 某版本)和 `conflicts`(与某 uid 互斥),合并前做依赖求解。

> Prism `Component.h` / `PackProfile.h` / `LaunchProfile.h` / `VersionFile.h` 是完整参考;PCL 系看 `inheritsFrom` 递归解析。

## 7. 自研要点

1. **先支持 Mojang 格式**(原版 + Fabric/Forge 生成的 json 都是它),再考虑自定义组件格式。
2. **Rule 求值单独抽一个函数 + 一个 RuntimeContext**(OS/arch/javaVersion/features),库过滤、参数过滤、native 选择都复用它。
3. **GradleSpecifier 抽成一个类**,负责坐标 ↔ 路径 ↔ URL 转换。
4. **合并产物 LaunchProfile 要可序列化**,方便调试"最终到底用了哪些库、什么 mainClass"。
5. loader 安装本质就是「拿到 loader 的版本 json + 它需要的额外库」,装好后它就是普通的可合并组件。
