# 09 · 工程参考(框架 / 模块 / 存储层)

> 给后续工程优化用的"现状底图"。记录:用了哪些框架/库、所有模块职责、**每一处数据存在哪、怎么存**、以及已知的优化点。
> 由代码审计自动生成核对(2026-06),与实现一致。配套:[04 施工蓝图](./04-rust-tauri-design.md)、[08 数据层+lite服务器](./08-data-layer-and-lite-server.md)。

---

## 一、技术栈 / 框架(按 crate)

```
crates/mc-types   纯数据 DTO(无逻辑)        — serde
crates/mc-core    UI-free 启动器内核(46 模块) — tokio + reqwest + ...
crates/mc-cli     headless CLI(驱动/验证内核) — clap
crates/mc-server  Lite 后端(axum)            — axum + axum-login
desktop/src-tauri Tauri v2 胶水               — tauri
desktop/ui        前端                        — SolidJS + Vite
```

### mc-core(内核)依赖
| 库 | 版本 | 用途 |
|----|------|------|
| `tokio` | 1 | 异步运行时,驱动网络/文件 IO + 线程池并发 |
| `reqwest` | 0.12 | HTTP 客户端(`rustls-tls` + `gzip` + **`cookies`**),下载 + 调 API |
| `serde` / `serde_json` | 1 | 序列化;版本 json / 配置解析 |
| `thiserror` | 2 | 结构化错误类型 |
| `futures` / `tokio-util` | 0.3 / 0.7 | Future 组合子 / 流处理 |
| `rayon` | 1 | 并行批量 SHA1 校验、mod 扫描 |
| `sha1` / `sha2` / `md-5` / `hex` | 0.10 / 0.4 | 文件完整性校验 + 哈希编码 |
| `zip` / `tar` / `flate2` | 2 / 0.4 / 1 | jar/modpack 解压、natives 提取、tar.gz(JRE) |
| `fastnbt` | 2 | level.dat NBT 解析(世界元数据) |
| `trash` | 5 | 删除到系统回收站(安全删除) |
| `reflink-copy` | 0.1 | COW 克隆,加速实例复制 |
| `fs4` | 0.9 | 磁盘可用空间检测 |
| `dirs` | 5 | 跨平台数据目录解析 |
| `tracing` | 0.1 | 结构化日志 |

### mc-server(Lite 后端)依赖 🔑
| 库 | 版本 | 用途 |
|----|------|------|
| `axum` | **0.8** | HTTP web 框架(REST 端点);better-auth 要求 0.8(已从 0.7 升级) |
| **`better-auth`** | **0.10** | **登录框架(@better-auth 的 Rust 移植):email/password + session,内置社交/passkey/2FA/组织/API key/设备码等插件按需开。`SqlxAdapter::from_pool` 共享 Supabase 池,`axum_router()` 挂载。密码 argon2id** |
| **`sqlx`** | **0.8** | **持久化:Supabase(Postgres),运行时查询(编译期不需现成 DB);share/db 直接用** |
| `dotenvy` | 0.15 | 加载 `.env`(DATABASE_URL / AUTH_SECRET) |
| `tower-http` | 0.6 | CORS + trace 中间件 |
| `reqwest` | 0.12 | 后端调上游(Fabric/Quilt/Forge/NeoForge 元数据) |

### mc-cli:`clap` 4(参数解析)+ `anyhow`(错误)+ tokio
### desktop/src-tauri:`tauri` 2 + `tauri-plugin-shell` 2 + path 依赖 mc-core/mc-types
### desktop/ui:`solid-js` ^1.8 + `@tauri-apps/api` ^2 + `vite` ^5 + `vite-plugin-solid` + TypeScript ^5

---

## 二、模块地图(mc-core,46 模块)

| 区 | 模块 | 职责 | 关键 pub |
|----|------|------|----------|
| **基础** | `error.rs` | 结构化错误 + 路径上下文 | `CoreError`,`Result`,`IoResultExt` |
| | `fs.rs` | 文件名净化、问题路径检测(`!`/中文)、原子写、跨盘移动、廉价共享(hardlink/reflink/copy) | `write_atomic`,`share_file`,`sanitize_filename`,`check_problematic_path`,`safe_join` |
| | `paths.rs` | 目录模型 + 便携性:数据目录解析、多根发现、`GamePaths` 布局 | `resolve_data_dir`,`discover_roots`,`GamePaths` |
| | `settings.rs` | 全局设置(下载源/并发/内存/Java/语言/自定义根) | `GlobalSettings`,`mirror_resolver` |
| **版本** | `version/format.rs` | Mojang 版本 json 强类型(1.13+ args + 旧字符串 + inheritsFrom) | `VersionJson`,`Argument`,`AssetIndexRef` |
| | `version/profile.rs` | 合并继承链(vanilla+loader)→ `LaunchProfile` | `LaunchProfile`,`from_chain` |
| | `version/library.rs` | 库解析:Maven 坐标、natives 选择、平台规则过滤 | `Library`,`classpath_libraries`,`select_native_libraries` |
| | `version/rule.rs` | 规则求值(OS/arch/feature) | `Rule`,`RuntimeContext`,`rules_allow` |
| | `version/gradle.rs` | Maven 坐标 ↔ 路径/URL | `GradleSpec` |
| **下载** | `download/mod.rs` | 并发下载引擎:流式+增量 SHA1、原子落盘、退避重试、幂等跳过 | `Downloader`,`DownloadItem`,`download_all`,`get_json` |
| | `download/mirror.rs` | 镜像 URL 改写(BMCLAPI/官方) | `MirrorResolver` |
| | `download/checksum.rs` | SHA1 校验 + rayon 并行找坏文件 | `verify_sha1`,`find_broken` |
| **Java** | `java/detect.rs` | 探测已装 JRE(PATH/JAVA_HOME/系统) | `detect_all`,`probe`,`JavaInstall` |
| | `java/version.rs` | 解析 java 版本号 | `JavaVersion` |
| | `java/install.rs` | Adoptium JRE 自动下载安装 | `install_java` |
| | `java/mod.rs` | MC 版本 → 所需 Java major + 选择 | `required_major`,`select` |
| **账号** | `auth/offline.rs` | 离线账号(用户名 → 稳定 UUID) | `offline_session` |
| | `auth/msa.rs` | 微软 OAuth2 设备码 → XSTS → MC | `MsaClient`,`authenticate` |
| | `auth/store.rs` | 多账号持久化(JSON) | `AccountStore`,`StoredAccount` |
| | `auth/yggdrasil.rs` | 外置登录(authlib-injector) | `YggdrasilClient` |
| **启动** | `launch/command.rs` | 拼 java 命令行(占位符替换 + classpath) | `build_launch_command`,`LaunchVars` |
| | `launch/mod.rs` | 启动管线编排:解析→补文件→natives→Java→起进程 | `launch`,`LaunchSpec`,`install_version`,`ensure_files` |
| **加载器** | `loader/{fabric,quilt}.rs` | meta API 取 profile json,inheritsFrom 合并 | `install_fabric`,`install_quilt` |
| | `loader/{forge,neoforge}.rs` | 官方 installer 无头运行 | `install_forge`,`install_neoforge` |
| | `loader/installer.rs` | 通用 installer 执行 + 检测产物 | `run_installer` |
| **元数据** | `meta/mod.rs` | 拉 Mojang manifest/版本json/asset index → 下载项 | `fetch_manifest`,`library_download_items`,`asset_download_items` |
| **实例** | `instance/mod.rs` | 实例抽象(versions/<id>/)+ 枚举 | `Instance`,`list_instances` |
| | `instance/config.rs` | 实例级配置 | `InstanceConfig` |
| | `instance/lifecycle.rs` | 复制/删除(回收站)/`.mrpack` 导入导出 | `copy_instance`,`delete_instance`,`import_mrpack`,`export_mrpack` |
| | `instance/mods.rs` | 扫描列出 mod + 元数据 | `list_mods`,`ModInfo` |
| | `instance/install_mod.rs` | 从 Modrinth 装 mod + 依赖解析 | `install_mod`,`InstallReport` |
| | `instance/world.rs` | 世界列表(level.dat)+ 备份/删/重命名 | `list_worlds`,`WorldInfo` |
| | `instance/packs.rs` | 资源包/光影/数据包管理 | `list_packs`,`PackKind` |
| **Mod 平台** | `modplatform/modrinth.rs` | Modrinth API 客户端 | `ModrinthApi`,`search`,`get_versions` |
| | `modplatform/mod.rs` | 平台无关数据模型 | `SearchHit`,`ProjectVersion`,`ResourceKind` |
| **诊断** | `diagnostics.rs` | 崩溃日志关键词分析 | `analyze`,`CrashAnalysis` |
| **后端客户端** | `server.rs` | 调我们 lite server(loaders/news/share/登录) | `ServerClient`,`Profile` |

**mc-server**:`main.rs`(axum app/路由)· `auth.rs`(axum-login Backend)· `meta.rs`(加载器聚合)· `share.rs`(分享)· `news.rs`(新闻)
**mc-types**:`lib.rs`(共享 DTO)· `platform.rs`(OS/Arch 探测)

---

## 三、存储 / 持久化层 🔑(优化重点看这里)

> `$DATA_DIR` = `paths::resolve_data_dir()`(默认 OS app-data/`mc-launcher`,便携模式为 exe 旁 `launcher-data/`)。
> `$GAME_ROOT` = 某个游戏根目录(`.minecraft` 式)。

### 客户端(mc-core / Tauri)
| 数据 | 机制 | 位置 | 格式 | 优化点 |
|------|------|------|------|--------|
| **账号** | JSON + 原子写 | `$DATA_DIR/accounts.json` | `StoredAccount`(含 access_token/refresh_token) | ⚠️ **token 明文落盘** → 改 OS keyring(Keychain/DPAPI/libsecret),文件只留非敏感元数据 |
| 全局设置 | JSON + 原子写 | `$DATA_DIR/settings.json` | `GlobalSettings` | 无(缺失/损坏回退 Default) |
| 实例配置 | JSON + 原子写 | `$GAME_ROOT/versions/<id>/instance.json` | `InstanceConfig` | 无 |
| 主题 | JSON + **直接 write** | `$DATA_DIR/theme.json` | `ThemeConfig` | ⚠️ 改 `write_atomic`(当前非原子) |
| 版本 json | 原始文本 + 原子写 | `versions/<id>/<id>.json` | Mojang 原始 JSON(不重序列化) | 无(保留原字节兼容字段演进) |
| asset 索引 + 对象 | 流式下载 + 增量 SHA1 + 原子 rename | `assets/indexes/<id>.json` + `assets/objects/<2>/<hash>` | JSON / 二进制内容寻址 | ⚠️ **无 HTTP 缓存(无 ETag/断点续传)** |
| library jar | 同上 | `libraries/<maven 路径>` | jar 二进制 | 幂等跳过(存在且 SHA1 匹配即跳);⚠️ 无 HTTP 缓存 |
| launcher_profiles | 原子写 | `$GAME_ROOT/launcher_profiles.json` | 最小骨架(Forge/legacy 安装器要) | 无 |
| Java 安装 | 下载 + 解压(tar.gz/zip) | `$DATA_DIR/jre-<major>/` | JRE 归档 | 幂等(已存在即用);⚠️ 无断点续传 |
| mods/worlds/packs | 文件系统目录 + copy_dir/override_folder | `$GAME_ROOT/{mods,saves,resourcepacks,config}/` | 纯文件 | ⚠️ 无增量同步;多实例共享靠 `share_file`(hardlink/reflink) |
| 下载临时文件 | `.part` + fsync + rename | 各下载目录 `*.part` | 二进制流 | ⚠️ 断电会留 `.part`(应定期清理);不读 ETag/Last-Modified |
| Java 探测 | 不存储(运行时探测) | PATH/JAVA_HOME | `JavaInstall` | 无 |

### 服务端(mc-server)—— ✅ 已上 **Supabase(Postgres)**,真机验证
> 一个 `PgPool`(sqlx)撑起用户/会话/分享三表。连接串走 `DATABASE_URL`(Supabase Session pooler URI),
> 放在 `crates/mc-server/.env`(gitignored,`.env.example` 是模板)。建表见 `db.rs`(`CREATE IF NOT EXISTS`),首次连接自动建。
> **当前 dev 项目**:Supabase `mc-launcher-dev`(ref `lxjwwuexdyulkgjpzgjk`,us-east-1,Postgres 17.6)。
> 验证:register/login=200,行落库(argon2id 哈希,非明文),`me` 会话往返通过(`tower_sessions.session` 持久化)。

> 登录由 **better-auth** 接管(端点 `/v1/auth/sign-up/email`、`/sign-in/email`、`/get-session`、`/sign-out`,nest 在 `/v1/auth`)。
> 它拥有 `users/sessions/accounts/verifications` 四张表(schema 见 `db.rs` 内嵌的 better-auth 001 迁移);密码以 **argon2id** 存在 `users.metadata.password_hash`(`accounts` 表留给 OAuth/社交)。

| 数据 | 机制 | 位置 | 格式 | 优化点 |
|------|------|------|------|--------|
| **用户账户** | **better-auth + Postgres(sqlx)** `public.users` | Supabase(`$DATABASE_URL`) | id/email/name/username…;密码 = `metadata.password_hash`(argon2id) | ✅ 持久化,跨重启验证 |
| **会话** | **better-auth** `public.sessions` + httponly cookie | Supabase | DB 行 + session cookie | ✅ 持久化(重启不掉登录);`sign-out` 真失效 |
| OAuth/社交账号 | **better-auth** `public.accounts` | Supabase | provider_id/access_token/… | 现为空,接社交登录插件时用 |
| 实例分享 | **Postgres(sqlx)** `public.shares` | Supabase | `id(FNV-1a 内容哈希,确定性) + json` | ✅ 已持久化 |
| 新闻 | 硬编码函数 | `news::feed()` 返回 | `NewsItem`(当前 2 条样本) | ⚠️ 仍占位 → CMS/文件 |

> 启动:`crates/mc-server/.env` 填好 `DATABASE_URL` 后 `cargo run -p mc-server`(自动加载 .env)。
> 用 sqlx **运行时查询**(非 `query!` 宏),编译期无需现成数据库。
> 切其它 Postgres(或正式库)= 换 `DATABASE_URL` 即可,代码零改动。Session pooler 注意用 **aws-N 区号正确的 host**(CLI link 后见 `supabase/.temp/pooler-url`)。

---

## 四、工程优化清单(从上表提炼,按优先级)

### P0 安全
- [ ] **账号 token 明文** → 系统 keyring(`accounts.json` 只留 uuid/username/owns_game)。涉及 `auth/store.rs`。

### P1 服务端生产化
- [x] ✅ **用户/会话/分享 → Postgres(sqlx),已上 Supabase**(项目 `mc-launcher-dev`),真机验证。`db.rs` 建表,`auth.rs`/`share.rs` 是唯一碰 SQL 的地方。
- [ ] 加 `sqlx::migrate!` 迁移框架(当前是 `CREATE IF NOT EXISTS`,schema 演进时需要)。
- [ ] **新闻 → CMS/文件**(仍硬编码占位)。
- [ ] session cookie `secure=false`(本地 http);生产要 https + `with_secure(true)`。
- [ ] dev DB 密码在 `.env`(gitignored);正式部署用密钥管理(不入库)。Supabase 直连 host `db.<ref>.supabase.co` 新项目已无 DNS,统一走 Session pooler(IPv4)。

### P2 下载层
- [ ] **HTTP 缓存**:加 `HttpMetaCache`(ETag/Last-Modified + 条件 GET),减少 manifest/版本 json 重复拉取。现在完全没有缓存层。
- [ ] **断点续传**:大文件(client jar / JRE)支持 Range 续传;现在断了重下整个。
- [ ] `.part` 临时文件孤儿清理(启动时扫一遍)。

### P3 性能/体验
- [ ] 客户端列表类数据(实例/mod 缓存索引)规模大时上 **SQLite**(现在全是 JSON 文件 + 每次扫盘)。PCL-CE 用 SQLite 是参考。
- [ ] 主题写入改 `write_atomic`(P3 小补)。
- [ ] modpack overrides 增量同步(现在每次全量 copy)。

### 不算债的设计选择(别误改)
- 版本 json 存原始字节(兼容字段演进)— 故意的。
- 下载幂等跳过(SHA1 匹配即跳)— 让整批可安全重试。
- 实例间共享走 hardlink/reflink(`share_file`)— 省磁盘,故意的。
- 设置/实例配置缺失即回退 Default — 故意的容错。
