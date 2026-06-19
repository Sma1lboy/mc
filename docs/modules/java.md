# 模块 · Java 管理

> 找到/下载一个能跑目标版本的 JRE,并匹配正确的 major 版本。

## 1. 版本匹配规则

| MC 版本 | 需要 Java |
|---------|-----------|
| ≤ 1.16.5 | Java 8 |
| 1.17 ~ 1.20.4 | Java 17 |
| ≥ 1.20.5 | Java 21 |

版本 json 的 `javaVersion.majorVersion` 字段直接给出推荐值,优先用它;没有则按上表兜底。

## 2. 本机 Java 检测

多种扫描器组合(PCL-CE 用了 5 种 scanner):

| 来源 | 方法 |
|------|------|
| 注册表(Windows) | 扫 `HKLM\SOFTWARE\JavaSoft\...`、各 JDK 厂商键 |
| PATH 环境变量 | 遍历 PATH 找 `java(.exe)` |
| `where` / `which` | 系统命令查询 |
| 常见安装路径 | Program Files、`/usr/lib/jvm`、`~/.sdkman` 等 |
| Microsoft Store Java | UWP 安装的 Java |
| 启动器自带目录 | 之前自动下载到实例/全局目录的 JRE |

对每个候选执行 `java -version`(或跑一个打印 system properties 的探测程序),解析:
- 版本号(major/minor/patch)
- 位数(32/64 bit)
- 架构(x64/arm64)
- 厂商

> Prism `java/JavaChecker`(跑探测程序拿结构化结果);PCL-CE `PCL.Core/Minecraft/Java/JavaManager` + 多 scanner。

## 3. Java 自动下载

本地无匹配版本时:
- **Mojang java runtime**:Mojang 提供 java runtime manifest,按平台下载官方打包的 JRE(jre-legacy / java-runtime-gamma / delta 等)。BMCLAPI 有镜像。
- **第三方**:Adoptium(Temurin)、Azul Zulu、Microsoft OpenJDK、Amazon Corretto 的 API。
- 下载后解压到实例 `java/` 或全局 java 目录,再走一遍检测验证。

> Prism `minecraft/launch/AutoInstallJava` + `java/download/`。

## 4. 选择策略

- 自动:在所有已知 Java 里选 major 匹配且架构匹配(优先 64 位、优先与系统架构一致)的。
- 手动:允许用户给实例指定固定 Java 路径(覆盖自动)。
- 全局默认 + 实例覆盖两级配置。

## 5. 自研要点

1. **检测做成带缓存的服务**:扫描慢,结果缓存到数据库/配置,启动时不重复扫。
2. **探测用一个小 Java 程序打印 properties** 比解析 `java -version` 文本更可靠(后者格式各厂商不一)。
3. **arm64 Mac / arm64 Windows 要特判**——native 库和 Java 架构必须一致。
4. 自动下载优先走镜像(国内),并校验解压后的完整性。
