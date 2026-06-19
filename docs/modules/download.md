# 模块 · 下载系统

> 启动器 80% 的"卡顿/失败"投诉都来自下载。这是最值得投入工程的模块,也是 PCL 系流行的核心。

## 1. 分层抽象(参考 Prism net/)

```
Task (异步任务基类: 状态机 + 进度 + 可中止 + 信号)
 ├─ NetRequest / Download        单个 HTTP 请求
 │     ├─ Sink                   响应数据去哪:FileSink(写文件) / ByteArraySink(内存) / MetaCacheSink(缓存+文件)
 │     └─ Validator              校验:ChecksumValidator(MD5/SHA1)
 ├─ NetJob (ConcurrentTask)      一批下载,带并发上限
 └─ SequentialTask               必须顺序执行的任务链
```

要点:
- **Sink 模式**:把"下载"和"数据落地"解耦。同一个 Download 既能写文件也能进内存,复用度高。
- **Validator 链**:下载完跑校验,不过则判失败 → 重试或报错。
- **进度聚合**:NetJob 聚合所有子任务进度,UI 显示总进度。

## 2. 并发与限速

- **并发下载**:线程池/异步并发,默认几十个并发(PCL 默认 63 线程)。可配上限。
- **限速**:调度器按配置的 KB/s 限制总速率,避免占满带宽影响其他应用(国内用户很在意)。
- **大文件分片**(可选):单文件分 range 多线程下载,小文件不分。

## 3. 多源镜像(国内核心)🌟

PCL 系流行的根本。三种做法:

1. **源切换**:全局配置当前源(0=官方 / 1=BMCLAPI / …),所有 URL 按源生成。
2. **URL 改写**:把 Mojang URL 透明替换成镜像 URL。
   ```
   https://piston-data.mojang.com/...  →  https://bmclapi2.bangbang93.com/...
   https://libraries.minecraft.net/... →  https://bmclapi2.bangbang93.com/maven/...
   ```
3. **多源并联/竞速**:官方 + 镜像同时发请求,谁先返回用谁,另一个取消。最稳但费流量,通常只对小的元数据请求用。

BMCLAPI 能镜像的:version manifest、版本 json、client jar、libraries、assets、Forge/Fabric/OptiFine 安装器、Java runtime。

## 4. HTTP 缓存(减少重复下载)

参考 Prism `HttpMetaCache`:
- 对元数据类资源(version manifest、meta 索引)用 **ETag + Cache-Control max-age**。
- 本地存缓存条目(etag、过期时间、本地路径),请求时带 `If-None-Match`,304 就用本地。
- 缓存目录按类型分:`caches/library`、`caches/assets`、`caches/meta`。

## 5. 文件校验与补全(DlClientFix 模式)🌟

启动器的"修复游戏文件"功能本质就是全量校验 + 补全:

```
对实例的每个应有文件(client jar / 每个 library / assetIndex / 每个 asset object):
  FileChecker.check(path):
    - 不存在            → 缺失,加入下载队列
    - 大小不符          → 损坏,删除重下
    - SHA1 不符(若有)  → 损坏,删除重下
  → 把所有缺失/损坏项组成一个 NetJob 并发下载
```

PCL 的 `DlClientFix` 把 libraries / assetIndex / assets / java runtime 分别补全,assets 可后台异步补(不阻塞启动)。

## 6. 各实现对应

| 实现 | 关键文件 |
|------|----------|
| Prism | `net/NetJob`、`net/NetRequest`、`net/Download`、`net/FileSink`、`net/Validator`、`net/HttpMetaCache`、`net/MetaCacheSink` |
| PCL-CE | `PCL.Core/IO/Download/`(NDlFactory + IDlConnection + IDlWriter)、`PCL.Core/IO/Net/Http/HttpRequest`、`IO/Net/Dns`(DNS 竞速/DoH) |
| PCL2 | `Modules/Base/ModNet.vb`(2010 行,重试/代理/多源)、`Modules/Minecraft/ModDownload.vb`(DlSourceLoader 多源、FileChecker、DlClientFix) |

## 7. 自研要点

1. **先把 Task/Sink/Validator 三件套搭好**,所有网络操作(下载文件、调 API、刷 token)都走它。
2. **镜像源做成可插拔的 URL 改写器**,一个函数 `rewrite(url, source) -> url`,集中管理。
3. **校验和补全是同一套逻辑**——平时启动顺手校验,"修复"按钮就是强制全量校验。
4. **重试要带退避**(指数退避 + 最大次数),并区分"可重试错误"(超时/5xx)和"不可重试"(404)。
5. **assets 下载量大但可后台**,libraries/jar 必须前台完成才能启动。
6. DNS 层在国内值得做:DoH + 多 DNS 竞速能显著降低"解析失败/被污染"(PCL-CE 的 `WeightedDnsRacerClient`)。
