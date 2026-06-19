# 模块 · 下载系统

> 启动器 80% 的"卡顿/失败"投诉都来自下载。这是最值得投入工程的模块,也是 PCL 系流行的核心。
>
> 本文是对 `ref/` 三个启动器下载子系统的**逐文件**梳理 + 对当前 `mc-core` 实现的差距对照,
> 作为下载模块演进的施工蓝图。引用均标注了 `ref/` 源码的 `file:line`。

## 0. 现状速览(mc-core)与差距地图

当前 `crates/mc-core/src/download/` 已经把**单文件管线**做得很对(对齐 Prism 的 `FileSink + ChecksumValidator`):

- `mod.rs` — `Downloader` + `DownloadItem{url, path, sha1, size}`;流式写 `.part` + 增量 sha1 + 原子 rename + 幂等跳过 + 指数退避。
- `checksum.rs` — 仅 sha1。
- `mirror.rs` — `MirrorResolver`:纯前缀改写,**单条** URL → 单条 URL,只认 BMCLAPI 游戏域名。

差距(按 ROI 排序,详见 §11):

| # | 缺口 | 影响 | 参考 |
|---|------|------|------|
| P0 | `DownloadItem` 只有**单 url**,无候选源列表 → 零故障转移 | 镜像挂了整项就失败 | PCL2 `NetFile.Sources` |
| P0 | `MirrorResolver.rewrite` 返回**单条**,只有 BMCLAPI、无 McIM | 社区 mod 下载无国内镜像 | PCL2 `DlSource*` |
| P0 | `download_all` **首错即整批中止**,无作业级重试、无"尽力补全"结果 | 补文件场景不可用 | Prism `NetJob` 3 趟重投 |
| P1 | 仅 sha1;无 sha512 / md5 / murmur2 | .mrpack(sha512)、CurseForge(md5/指纹)无法校验 | §5 |
| P1 | `is_retriable` 把所有 `is_status()`(含 404/403)当可重试 | 浪费 + BMCLAPI 限流被误杀 | §2.3 |
| P1 | 无 ETag / If-None-Match 元数据缓存 | 每次重拉 manifest/平台 JSON | Prism `HttpMetaCache` |
| P2 | `Progress.speed_bps` 恒为 0、只报"完成项/总项",无字节级进度 | UI 无速度/ETA | PCL2 加权测速 |
| P2 | 无本地去重(不扫其它 `.minecraft` 复制同 hash 文件) | 重复下载共享库/资源 | PCL2 `CheckExistingFiles` |
| P3 | 无磁盘空间预检、无单连接 stall 超时、无限速、无分片续传 | 弱网/大文件边界 | §8 §9 |

> 总结:**单文件正确性已达标,缺的是"多源 + 作业级韧性 + 多哈希 + 缓存"这一层。** 这正是
> 把 mc-core 从"能下"提升到"国内能稳下"的关键,也是后续整合包导入/导出的前置依赖。

## 1. 分层抽象(参考 Prism `net/`)

Prism 的下载栈是三层解耦,值得照搬其**形状**:

```
Task (异步任务基类: 状态机 + 进度 + 可中止 + 信号)
 ├─ ConcurrentTask              通用并发执行器:queue + doing/done/failed 集合,固定 max_concurrent
 │   └─ NetJob                  "一批下载",在 ConcurrentTask 上叠加 3 趟重投失败项(跳过 404)
 ├─ NetRequest / Download       单个 HTTP 请求的状态机(重定向、429、流式)
 │     ├─ Sink                  数据落地策略:FileSink(QSaveFile 临时文件 + 原子 commit) /
 │     │                        ByteArraySink(内存) / MetaCacheSink(FileSink + ETag 缓存)
 │     └─ Validator             边写边校验:ChecksumValidator(可插拔 MD5/SHA1/SHA512)
 └─ SequentialTask              必须顺序的任务链
```

要点(`ref/PrismLauncher/launcher/net/`):

- **Sink 模式**(`FileSink.cpp`):把"下载"与"数据落地"解耦——同一个 `Download` 既能写文件也能进内存。
  `FileSink.write()` 是 `writeAllValidators(data)` **然后** 写盘;`finalize()` 校验通过后 `commit()`(原子 rename)。
- **Validator 链**(`ChecksumValidator.h`):`init` 重置哈希,`write` 喂字节,`finalize` 比对;`m_expected` 为空即跳过。
- **gotFile 排除 304**(`FileSink.cpp`,`gotFile = status==200||203`):条件 GET 命中缓存(304)不能截断/覆盖已有文件。
- **mc-core 对照**:`Downloader::fetch_to_part` ≈ `NetRequest`+`FileSink`+`ChecksumValidator` 三合一(流式写 `.part`、
  增量 sha1、`rename`)。已经做对,只是没拆成可插拔的 Sink/Validator——目前**够用**,等需要内存下载/多哈希时再拆。

## 2. 单文件下载管线

### 2.1 流程(mc-core 现状,已正确)

```
download_one(item):
  幂等跳过: 目标已存在且 sha1 匹配 → 直接返回
  镜像改写: url = mirror.rewrite(item.url)
  建父目录
  循环 MAX_ATTEMPTS:
    fetch_to_part(url, .part)            # 流式写 + 增量 sha1
    校验 sha1(若有) 不符 → 删 .part, 返回 Checksum 错误(不重试)
    rename(.part → 最终路径)             # 同 FS 原子
    成功
  否则: 网络/IO 错误 → 删 .part, 指数退避重试
```

- **原子落盘**:Prism 用 `QSaveFile`(写 `.XXXXXX` 临时再 `commit`),mc-core 用 `<path>.part` + `rename` —— 等价且更简单,**保留**。
  若以后会并发扫描实例目录,可学 Prism 维护一个"在飞临时路径"集合,避免扫到半成品。

### 2.2 断点续传 / 范围请求(缺,P3)

- Prism 的元数据走 `HttpMetaCache`(条件 GET);PCL2 走真正的**字节级续传**:首线程 plain GET 学到 `Content-Length`,
  后续线程对最大未完成区间发 `Range: bytes=start-` 写各自 `.tmp`,死线程的剩余区间被新线程从 `start+done` 接管,最后 `Merge` 拼接。
- **mc-core**:`.part` 失败即整文件重下,无 Range。短期可不做(只影响弱网大 jar);若做,照 PCL-CE 的干净接口
  (`NDlConnectionInfo{Length, BeginOffset, EndOffset, IsSupportSegment}` + writer 的 `FinishAsync` 合并),用 reqwest 发 Range GET 写 `.partN` 再拼。
- **必须检测"不支持 Range"**(PCL2):Range 请求若返回完整 `Content-Length`(== 文件大小)或 416,该源不支持分片,退回单线程——
  否则会写坏文件。

### 2.3 重试策略与状态码(P1)

当前 `is_retriable`(`mod.rs:302`)把所有 `e.is_status()` 当可重试,**过粗**。照 Prism/PCL2 收紧:

- **404 绝不重试**(Prism `NetJob.executeNextSubTask` 用 `removeIf` 排除 404;PCL2 `SourceFail` 直接禁用源)。
- **4xx(除 408/429)视为不可重试**。
- **429 特殊处理**(Prism `NetRequest::handleAutoRetry`):优先读 `Retry-After`(RFC1123 GMT 日期 *或* 整数秒),否则 `10 * 2^n`;
  `delay > 60s` 或 `> 4` 次则放弃。
- **BMCLAPI 的 403/429 不算硬失败**(PCL2 `ModNet.vb:1162`):镜像在高频下正常限流,误判会把最好的国内源拉黑(见 §4.4)。

## 3. 批量作业(作业级重试,P0)

当前 `download_all`(`mod.rs:241`)`while let Some(res) = stream.next() { res?; }` —— **首错即返回**,其余 in-flight 随 stream drop 取消。
对"补全缺失文件"这种**尽力而为**的场景是错的。照 Prism `NetJob`:

```
download_all(items) -> DownloadOutcome:
  并发跑全部(Semaphore + buffer_unordered)        # 已有
  收集失败到 Vec<(item, err)> 而非首错即返           # 改:不要 res?
  重投失败项(排除 404)最多共 3 趟                    # 加:NetJob.m_try<3
  返回 { succeeded, failed }                         # 加:让上层决定是否致命
```

- **单文件失败不应中止整批**(Prism/PCL2 都是累积失败、末尾才判)。
- 建议引入 `DownloadOutcome { succeeded, failed: Vec<(DownloadItem, CoreError)> }`,
  让"修复游戏文件"能尽量补、列出补不上的;而"安装版本必备库"仍可在有 failed 时整体失败。
- 并发上限应从用户设置读(Prism `NumberOfConcurrentDownloads`;PCL2 `ToolDownloadThread+1`),而非 `Downloader::new` 时固定。

## 4. 多源镜像(国内核心)🌟 — P0

> **结论先行**:Prism **不**做 Mojang 域名改写(它只有一个可配的 meta server),不是国内启动器的参照。
> 要照 **PCL2/PCL-CE**:**每个逻辑文件持有一个有序的候选 URL 列表**,下载器在候选间故障转移并按源健康度降级。
> 这是 PCL 全部下载韧性的根。

### 4.1 把"单 url"升级为"候选列表"

- 给 `DownloadItem` 增加 `urls: Vec<String>`(有序候选),或新增 `MultiSourceItem`。
  `download_one` 依次尝试:`url[0]` 在"**可重试且非校验失败**"时切 `url[1]`……每个 URL 各自跑完重试再前进。
- `MirrorResolver::rewrite(&str)->String` 升级为 `candidates(official_url, kind: ResourceKind) -> Vec<String>`,
  返回有序列表 `[可能的官方, mirror1, mirror2…]`。

参照 PCL2 数据结构(`ref/PCL2/.../ModNet.vb`):

| 类型 | 字段 | 作用 |
|------|------|------|
| `NetSource` | `Id, Url, FailCount, Ex, SingleThread, IsFailed` | 一个候选 URL + 健康状态(失败即降级的单位) |
| `NetFile` | `Sources, SourcesOnce(单线程兜底源), Retried, FileSize, Check:FileChecker, …` | 一个逻辑文件,持有有序候选 + 故障转移簿记 |
| `NDlSourceReport`(PCL-CE) | `MaxSegmentCount, RetryCount, AverageSpeed` | 最干净的"源健康度"蒸馏,用于排序/降级镜像 |

### 4.2 按资源类型分别改写(BMCLAPI,游戏文件)

PCL2 的 `DlSource*`(`ModDownload.vb:1210-1311`)是**权威改写规则**,按 kind 不同:

```
asset:        resources.download.minecraft.net | piston-data | piston-meta
              → bmclapi2.bangbang93.com/assets(保留路径)
library:      host ∈ {minecraftforge, fabricmc, neoforged}  → 仅镜像(无官方回退!loader 不在 Mojang CDN)
              其它 → 官方 + 两个镜像变体 .../maven 和 .../libraries   ← BMCLAPI 两个路径都要给,某些 jar 只在其一
client jar /
assetIndex /
version json: piston-data | piston-meta | launcher.mojang.com | launchermeta.mojang.com
              → bmclapi2.bangbang93.com(根)
```

> mc-core 当前 `mirror.rs` 只发 `/maven` 一个变体、且无 loader-host 特判 → 部分 loader 工件会在镜像 404 且无备选。
> 改写前先做 `http→https` 归一(PCL2 `:1247`)。

### 4.3 社区 Mod 走 McIM(第二个镜像,P0)

mc-core 目前 `modplatform/modrinth.rs` 直连 Modrinth、**零国内镜像覆盖**。补 McIM(`DlSourceModGet`):

```
api.modrinth.com | staging-api.modrinth.com   → mod.mcimirror.top/modrinth
cdn.modrinth.com                               → mod.mcimirror.top
api.curseforge.com                             → mod.mcimirror.top/curseforge
edge/mediafilez/media.forgecdn.net             → mod.mcimirror.top
```

把它接进**两处**:平台 API 调用(search/version)和文件下载。

### 4.4 源排序、健康度与降级

- **排序偏好**:加 `enum DownloadSource { Auto, MirrorFirst, OfficialFirst }`(对齐 PCL2 `ToolDownloadSource`,国内默认 MirrorFirst)。
  `candidates()` 据此把官方/镜像谁放前。**文件下载**与**版本列表拉取**用各自的排序旋钮(PCL2 `DlSourceOrder` vs `DlVersionListOrder`,
  列表拉取对延迟敏感、值得竞速)。
- **降级**(PCL2 `SourceFail`):命中 404/416/502、Range 不支持、DNS 失败、空响应、或连续失败过多 → 标 `IsFailed` 跳过;
  **但 host 含 `bmclapi` 且 403/429 时不降级**(镜像限流是预期)。这一条特判最关键。
- **单线程重试兜底**(PCL2 `SourcesOnce`):所有候选常规模式各失败一次后,再把每个候选**逐个**单线程/无 Range 重试一遍——
  专治"镜像返回 200 但内容损坏/在并发下出错"。mc-core 当前对 sha1 不符直接放弃,应**至少切下一个源**(国内最常见的失败模式就是坏镜像)。
- **BMCLAPI 限速**:派发到 bmclapi host 后小延迟(~100ms)或限该 host 并发(按 hostname 的 per-host Semaphore)。
- 镜像 base URL 应**可配**(借 Prism `META_URL` 可覆盖的思路),`MirrorResolver::from_rules` 即定制缝。

### 4.5 暂缓:DNS/IP 健康度

PCL2 的 per-IP 可靠性打分 + 手动 IPv4/IPv6 选择(`DNSLookup`/`IPReliability`,绕 GFW 单栈封锁)很强但复杂,
且与 reqwest+系统解析部分重叠。**记为后续增强**;高 ROI 的是候选列表、按 kind 改写(含 McIM)、源降级、bmclapi 403/429 豁免。

## 5. 文件校验与补全(多哈希,P1)

### 5.1 多哈希支持

mc-core `checksum.rs` 硬编码 sha1,但生态需要多种:

| 哈希 | 用处 |
|------|------|
| sha1 | 原版 version json、库 |
| **sha512** | Modrinth `.mrpack` 索引(导入校验 + 导出反查的查询哈希) |
| **md5** | CurseForge 文件哈希之一 |
| **murmur2(变体)** | CurseForge 指纹(`/fingerprints` 反查;**滤掉字节 9/10/13/32**,seed=1,见整合包文档) |

建议:`download/checksum.rs` 增 `sha512_file()`;并引入 `enum Checksum { Sha1, Sha512, Md5 }` 让校验器按算法选取。
PCL2 `FileChecker` 还按**哈希串长度**自动判类型(32/33=MD5、40=SHA1、64=SHA256),可借鉴。

### 5.2 FileChecker(多准则校验)

PCL2 `FileChecker`(`ModBase.vb:663`)= `ActualSize + MinSize + Hash + CanUseExistsFile + IsJson`,`Check()` 返回错误字符串而非抛异常。
mc-core 可加 `download/check.rs::FileChecker`,统一"大小 + 任意哈希 + JSON 形状"校验,并为本地去重复用。

### 5.3 校验补全 = "修复游戏文件"

```
对实例每个应有文件(client jar / 每个 library / assetIndex / 每个 asset object):
  FileChecker.check(path): 不存在→缺失 / 大小不符→损坏 / hash 不符→损坏
  → 缺失+损坏项组成一个 (尽力而为的) 批下载
```

PCL2 `DlClientFix` 把 libraries / assetIndex / assets / java 分别补全,**assets 可后台异步补**(不阻塞启动);
libraries/jar 必须前台完成。这点配合 §3 的 `DownloadOutcome`(尽力补 + 列出补不上的)正好。

## 6. HTTP 元数据缓存(ETag,P1)

参考 Prism `HttpMetaCache` + `MetaCacheSink`:

- `MetaEntry{ md5sum, etag, local_changed_timestamp, remote_changed_timestamp(RFC2822), current_age, max_age, eternal, stale }`,
  索引是带版本号的 JSON,30s 批量保存。
- 请求带 `If-None-Match`(etag)/ `If-Modified-Since`;**304 复用本地**;`finalize` 解析 `Cache-Control max-age`/`Expires`/`Age` 设寿命。
- **mc-core**:`get_json`/`get_bytes` 每次重拉。加 `download/meta_cache.rs`:`{url → etag, last_modified, md5, max_age, fetched_at}`
  存版本化 JSON 索引;对 version manifest、平台 JSON 发条件 GET。**这是元数据侧 ROI 最高的一块。**

## 7. 本地去重(P2)

下载前先在已知的 `.minecraft` 目录(含官启的 `%APPDATA%/.minecraft`,HMCL 也存这)里找**同 size+hash** 的文件,
直接 copy / hardlink 而非下载(PCL2 `CheckExistingFiles`)。按路径类别(assets/libraries/versions/mods…)缩小扫描范围。
对共享库/资源是巨大带宽节省;mc-core 的幂等跳过只查目标路径本身,完全没这块。加 `download/dedup.rs`。

## 8. 进度、限速与边界(P2-P3)

- **字节级进度 + 加权测速**(P2):`download_one` 增可选"每文件字节回调",聚合到共享 `AtomicU64`,
  按 PCL2 的最近 ~30 样本加权(新样本权重高)、~100ms 刷新算速度,填 `Progress.speed_bps`(现恒为 0)。
- **限速**(P3):PCL2 全局令牌桶(`NetTaskSpeedLimitLeft` 每 100ms 由 `High/10` 补充)节流所有线程,用户可配上限。
- **磁盘空间预检**(P3):>50MB 文件下载前查临时盘(`size*1.1`)+ 目标盘(`size+5MB`)空间,早失败给清晰提示。
- **单连接 stall 超时**(P3):>5s 无字节且 <1KB/s 时丢弃该连接让监控重生(reqwest 无内建 idle-read 超时,需自己包)。

## 9. 分片 / 动态线程池(暂缓,P3)

PCL2 的 `NetThread` 链 + Range + 动态线程池(每 20ms 按全局速度阈值补线程)是最复杂的部分,只对慢网少数大 jar 有意义。
**先不做**;要做就照 PCL-CE 的接口而非 PCL2 的巨石。bmclapi/github/optifine 等**强制单线程**(它们不可靠地支持 Range)。

## 10. 各实现对应

| 实现 | 关键文件 |
|------|----------|
| Prism(引擎) | `tasks/ConcurrentTask.cpp`(并发执行器)、`net/NetJob.cpp`(3 趟重投)、`net/NetRequest.cpp`(429/重定向)、`net/FileSink.cpp`(原子 commit)、`net/ChecksumValidator.h`、`net/HttpMetaCache.cpp`(ETag) |
| Prism(镜像) | 不做域名改写;`CMakeLists.txt:204` 单一可配 `META_URL`;`modplatform/modrinth/ModrinthInstanceCreationTask.cpp:285`(整合包文件多 URL 队列故障转移) |
| PCL2 | `Modules/Base/ModNet.vb`(2010 行:NetSource/NetThread/NetFile、多源失败转移、单线程兜底、per-IP 评分、DNS)、`Modules/Minecraft/ModDownload.vb`(`DlSource*` 改写规则、`DlClientFix`)、`Modules/Base/ModBase.vb:663`(FileChecker) |
| PCL-CE | `PCL.Core/IO/Download/`(`NDlFactory` + `IDlConnection`/`IDlWriter`/`NDlConnectionInfo`/`NDlSourceReport`/`IDlResourceMapping`,接口化重构)、`IO/Net/Dns`(DNS 竞速/DoH) |
| mc-core | `download/mod.rs`(`Downloader`/`DownloadItem`/`download_one`/`download_all`)、`download/checksum.rs`(仅 sha1)、`download/mirror.rs`(`MirrorResolver`,单条改写) |

## 11. 自研要点 / 路线图

**P0(整合包生态前置依赖)**
1. `DownloadItem.urls: Vec<String>` 候选列表 + `download_one` 候选间故障转移。
2. `MirrorResolver::candidates(url, kind) -> Vec<String>`,移植 PCL2 BMCLAPI 规则(库的 `/maven`+`/libraries` 双变体、loader-host 仅镜像)+ **新增 McIM** 社区 mod 镜像。
3. `download_all` 改为累积失败 + 3 趟重投 + 返回 `DownloadOutcome{succeeded, failed}`;并发度从设置读。

**P1**
4. 多哈希:`sha512_file()` + `Checksum` 枚举;为整合包导入/导出铺路。
5. 收紧 `is_retriable`:404/4xx 不重试、429 读 `Retry-After`、**bmclapi 403/429 豁免**;校验不符切下一个源。
6. `HttpMetaCache` 等价物:`download/meta_cache.rs` 条件 GET + 304 复用。

**P2**
7. 字节级进度 + 加权测速(填 `speed_bps`)。
8. 本地去重 `download/dedup.rs`(扫其它 `.minecraft` 复制同 hash 文件)。
9. `DownloadSource{Auto,MirrorFirst,OfficialFirst}` 偏好 + 源健康度/降级 + per-host 节流。

**P3**
10. 磁盘空间预检、单连接 stall 超时、全局限速。
11. 分片/Range 续传(照 PCL-CE 接口);DNS 竞速/DoH(对齐 PCL-CE `WeightedDnsRacerClient`)。

> 实现顺序建议:**P0 三件套**先落地(它们也是整合包导入的硬依赖),再补 P1 的多哈希 + 缓存,体验项(P2/P3)随后。
