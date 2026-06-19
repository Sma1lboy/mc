# 08 · 数据层补齐 + Lite 服务器(路线图)

> 方向(2026-06):暂不管 UI,优先把**本地数据层 + 所有值得做的 feature** 落地,
> 再落地我们自己的 **lite 服务器层**,并支持**本地测试环境**。
> 原则:每个 feature 配 CLI 命令可验证 + 单元/集成测试;不做镀金。

## A. 本地数据层 feature 清单(含「是否值得做」判定)

图例:✅ 做 · 🔶 之后做 · ❌ 不做(说明原因)

### A1 加载器(loader/)
| feature | 判定 | 说明 |
|---------|------|------|
| Fabric | ✅ 已完成 | meta 拉 profile json |
| Quilt | ✅ | 与 Fabric 同构(quilt meta API),几乎零成本 |
| NeoForge | ✅ | 当代 Forge 后继,主流;installer 处理 |
| Forge | ✅ | 仍是最大生态;installer profile 处理(较复杂) |
| OptiFine | 🔶 | Sodium 替代中,但仍有用户;作为 standalone/Forge 之上 |
| LiteLoader | ❌ | 已死 |

### A2 资源管理(resource/)
| feature | 判定 | 说明 |
|---------|------|------|
| Mod 安装进实例 | ✅ | 从 Modrinth 版本下载 jar 到 mods/,写元数据索引 |
| Mod 依赖解析 | ✅ | 递归解析 required deps,给可见预览 |
| 本地 Mod 管理 | ✅ | 扫描 mods/、启用禁用(.jar↔.disabled)、读 fabric.mod.json/mods.toml 元数据 |
| 资源包/光影/数据包 | ✅ | 列表 + 安装(从 Modrinth)+ 启用禁用 |
| 世界/存档管理 | ✅ | 列表(读 level.dat:名称/模式/最后游玩)、备份、删除、重命名 |
| 截图管理 | 🔶 | 列表/删除,低优先 |

### A3 实例生命周期(instance/)
| feature | 判定 | 说明 |
|---------|------|------|
| 创建(vanilla/loader) | ✅ | 已有安装链路,补「实例」抽象 |
| 复制 | ✅ | 用 fs::share_file 硬链共享 libraries/assets,秒级克隆 |
| 删除(回收站) | ✅ | 移入系统回收站(安全) |
| 导出/导入(.mrpack) | ✅ | Modrinth 整合包格式优先 |
| 版本隔离模式 | ✅ | PathIndie:saves/mods/logs 隔离到版本目录或共享根(配置) |
| 重命名/图标/分组 | 🔶 | 元数据,低优先 |

### A4 运行时与诊断
| feature | 判定 | 说明 |
|---------|------|------|
| Java 自动下载 | ✅ | Mojang java runtime manifest(+ BMCLAPI 镜像) |
| 崩溃日志分析 | ✅ | 关键词规则 → 人话原因(PCL 招牌) |
| log4j XML 日志解析 | 🔶 | 实时日志窗口用,UI 相关,延后 |
| 内存/磁盘预检 | ✅ | 已有 available_space;补内存检查 |

### A5 账号
| feature | 判定 | 说明 |
|---------|------|------|
| 微软 token 刷新接入启动 | ✅ | msa.refresh 已实现,接入 launch 的过期自动刷新 |
| 外置登录(Yggdrasil/authlib-injector) | ✅ | 国内私服/皮肤站刚需;authlib-injector javaagent 注入 |
| 离线 | ✅ 已完成 | |

### A6 配置与持久化
| feature | 判定 | 说明 |
|---------|------|------|
| 全局设置 | ✅ | 下载源/并发/Java/内存默认/镜像/语言,json 持久化 + 两级覆盖 |
| 实例 config | ✅ 已完成 | |
| 实例缓存索引 | 🔶 | 先 json;规模大再上 SQLite |
| 数据迁移(从官启/HMCL/PCL) | 🔶 | 提升迁移成本,之后做 |

### A7 不做 / 延后
- CurseForge API:需 key + 合规,🔶 延后(先 Modrinth 全覆盖)。
- 联机大厅/P2P:大工程,放到 lite server 之后单独立项。
- 自建下载镜像 CDN:基础设施重,先复用 BMCLAPI。

## B. Lite 服务器层(`crates/mc-server`)

定位:一个**轻量 Rust(axum)服务**,给启动器提供"官方源给不了 / 聚合更顺手"的能力。
launcher 通过一个可配置 base URL 指向它;本地测试时指向 `http://127.0.0.1:8787`。

### B1 v1 端点(值得做、可本地验证)
| 端点 | 作用 |
|------|------|
| `GET /v1/meta/loaders/:mc_version` | **聚合**:把 Fabric/Quilt/Forge/NeoForge/OptiFine 的可用版本归一成一个列表(省去客户端打多个上游) |
| `GET /v1/meta/versions` | 归一化的 MC 版本清单(可加我们的标注/推荐) |
| `GET /v1/news` | 启动器公告/新闻 feed(json) |
| `GET /v1/health` | 健康检查(本地测试探活) |
| `POST /v1/instances/share` / `GET /v1/instances/:id` | 实例/整合包分享(存元数据 + 文件清单,生成可分享 id) |
| **`POST /v1/auth/sign-up/email` / `sign-in/email` / `sign-out` / `GET /v1/auth/get-session`** | **启动器账号:走 `better-auth`(@better-auth 的 Rust 移植,功能最全的全家桶 auth)。email/password + 服务端 session(cookie),内置社交/passkey/2FA/组织/API key/设备码插件按需开。`SqlxAdapter` 共享 Supabase 池,`axum_router()` 挂载。密码 **argon2id** 存于 `users.metadata`。用户/会话/分享均持久化到 **Supabase(Postgres)**,跨重启验证通过。** ✅ 已落地 |
| `GET /v1/ping/:host` | MC 服务器状态查询代理(避免客户端直连协议复杂度) 🔶 |

### B2 暂不做
- 账号/OAuth 代理(隐私敏感,客户端直连微软即可)。
- 真·文件 CDN(基础设施)。

### B3 本地测试环境
- `cargo run -p mc-server` 起在 `127.0.0.1:8787`,内存态存储(无需 DB)。
- mc-core 增加 `ServerClient { base_url }`,默认指向我们的生产域名,测试时用 env `MC_SERVER_URL=http://127.0.0.1:8787` 覆盖。
- 集成测试:起服务 → mc-core 客户端打全部端点 → 断言。
- 给 CLI 加 `mc server-health` / `mc loaders <ver>`(走聚合端点)便于手测。

## C. 执行批次(workflow 编排)
```
Batch 1  loader/: quilt + neoforge + forge + optifine            (并行,各自文件)
Batch 2  resource/: mod 安装 + 本地 mod 管理 + 世界 + 资源包/光影/数据包
Batch 3  instance/: 复制/删除/导出导入(.mrpack) + 版本隔离
Batch 4  runtime: java 自动下载 + 崩溃分析 + 内存预检 + 全局设置
Batch 5  account: 外置登录(Yggdrasil) + 微软刷新接入
Batch 6  mc-server: axum 服务 + 端点 + ServerClient + 本地测试环境
```
每批:agent 写独立文件 → 我接线 lib.rs + 编译修复 → 加 CLI 验证命令 → 跑测试。
```
