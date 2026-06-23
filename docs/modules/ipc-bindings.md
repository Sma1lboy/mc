# IPC 契约 · Rust 单一真相生成(tauri-specta)

> 前端(WebView/SolidJS)与后端(Rust)是两个进程,只能经 Tauri 的 `invoke` 传 JSON 通信。
> 这份「IPC 契约」(命令名 / 参数 / 返回 DTO 的形状)以前两端各写一套,极易漂移。
> 现在由 **tauri-specta** 从 Rust 单方面生成 `desktop/src/ipc/bindings.ts`,前端跟随,杜绝漂移。

## 为什么(踩过的坑)

命令边界是「端到端无类型」的:`tsc` 不知道 Rust 的形状,手写的 `ipc/types.ts` 与 Rust DTO 不一致时
编译能过、构建能过,**只在运行时炸**。典型:Rust `SearchHit` 字段是 `id`,前端手写成 `project_id`,
于是 `hit.id === undefined` → `install_mod` 收到 `project: undefined` → 运行时 “missing required key project”。
另有一类是命令参数个数/可选性漂移(同样运行时才报)。

## 数据流

```
前端  api.modrinthSearch("sodium", "mod", mc, loader, null, null)
  │   commands.modrinthSearch(...) → invoke("modrinth_search", { query, kind, ... })   // 参数 JSON 化过桥
  ▼
Rust  #[tauri::command] #[specta::specta] fn modrinth_search(...) -> CmdResult<Vec<SearchHit>>
  ▼   返回 JSON 化过桥
前端  SearchHit[]
```

## 后端装配

- **依赖**(均为 rc,**升级需谨慎** —— bigint API 在补丁间挪过位置):
  - `crates/mc-types`、`crates/mc-core`:`specta = { version = "=2.0.0-rc.25", features = ["derive"] }`
  - `desktop/src-tauri`:同版 `specta`(再加 `function` 特性)+ `tauri-specta = "=2.0.0-rc.25"`(`typescript`)
    + `specta-typescript = "0.0.12"`
- **派生 `specta::Type`**:跨命令边界的 DTO 都派生它。
  - `mc-types`:全部(它本就是共享 DTO crate)。
  - `mc-core`:仅边界类型(`SearchHit`/`ModInfo`/`PackInfo`/`WorldInfo`/`ScreenshotInfo`/`ModUpdate`/
    `InstallReport`/`InstanceConfig`/`VersionDetail`/`ProjectDetail`/`GlobalSettings` 等)。内部类型不要派生
    (会把闭包拖大甚至编译失败)。
  - `commands.rs` 里的本地 DTO(`JavaDto`/`VersionInstallReport`/`DeviceCodeDto`/`BlockedFileDto`/`ImportOutcomeDto`)。
- **命令注解**:每个 `#[tauri::command]` 再加 `#[specta::specta]`。
- **`lib.rs` 装配**:用 `tauri_specta::Builder` 收集所有命令并在 debug 下导出绑定:
  ```rust
  let builder = Builder::<tauri::Wry>::new()
      .dangerously_cast_bigints_to_number()   // u64/i64(下载数/时间戳/字节)→ number,量级在 JS 安全整数内
      .commands(collect_commands![ commands::list_roots, /* …全部命令… */ ]);

  #[cfg(debug_assertions)]
  builder.export(
      specta_typescript::Typescript::default(),
      // 编译期锚定到 crate 目录:运行时 CWD 不定,相对路径会写错地方
      concat!(env!("CARGO_MANIFEST_DIR"), "/../src/ipc/bindings.ts"),
  ).expect("failed to export typescript bindings");

  tauri::Builder::default()
      // …
      .invoke_handler(builder.invoke_handler())   // 取代旧的 generate_handler!
      .run(/* … */);
  ```
- **生成时机**:`export()` 在 **debug 启动时**运行(非编译期)。即 `scripts/dev-app.sh` 跑一次 debug 应用即刷新
  `bindings.ts`。`bindings.ts` 已入库(前端依赖它编译,缺它则 `tsc` 挂)。

## 前端接线

- `desktop/src/ipc/bindings.ts`(生成,勿手改):导出 `commands` 对象(逐命令的强类型包装)+ 所有 DTO 类型。
- `desktop/src/ipc/types.ts`:**再导出**生成类型 + 历史命名别名:
  - `LoaderKind→Loader`、`VersionDetail→ModrinthVersion`、`ProjectDetail→ModrinthProject`、
    `DeviceCodeDto→DeviceCode`、`JavaDto→JavaInstall`、`ImportOutcomeDto→ImportOutcome`、`BlockedFileDto→BlockedFile`。
  - serde 序列化/反序列化形状不同的类型取「读取(反序列化)」形状:`InstanceConfig_Deserialize as InstanceConfig`、`PackInfo_Deserialize as PackInfo`。
  - 事件 payload(`InstallProgress`/`LaunchProgress`/`GameLog`/`GameStarted`/`GameExit`)、纯前端联合
    (`ProjectKind`/`ThemeMode`)仍手写 —— 它们不在命令签名里。`ThemeConfig` 由 `theme/theme.ts` 拥有(前端引擎要严格类型)。
- `desktop/src/ipc/api.ts`:`export const api` 是 `commands` 的 **Proxy**,把 specta 的
  `{status:"ok",data} | {status:"error",error}` 解包回项目既有的「成功 resolve / 失败 throw」约定;
  调用方 `await api.x(...)` + `try/catch` 不变。少数顺手 override:`exportModpack`(对象入参 vs 9 个位置参数)、
  `getTheme`/`setTheme`(边界处用 `theme.ts` 的严格 `ThemeConfig`)。事件订阅 `onXxx` 仍是手写 `listen`。

## 加一个新命令(流程)

1. Rust 写 `#[tauri::command] #[specta::specta] pub async fn foo(...) -> CmdResult<Bar>`;给 `Bar` 及其嵌套
   DTO 派生 `specta::Type`。
2. 把 `commands::foo` 加进 `lib.rs` 的 `collect_commands![ … ]`。
3. `scripts/dev-app.sh`(**全量**,非 `ui`)重启一次 → `bindings.ts` 自动多出 `foo` 与 `Bar`。
4. 前端直接 `await api.foo(...)`;若 `Bar` 要历史命名,在 `types.ts` 加一行别名。

## 注意

- **rc 依赖**:`specta*`/`tauri-specta` 锁死在 `=2.0.0-rc.25`;升级前先看 API 变更(尤其 bigint)。
- **生成靠 debug 启动**:改了 Rust DTO/命令后,务必跑一次 debug 应用刷新 `bindings.ts`,否则前端类型滞后。
- **别只跑 `dev-app.sh ui`**:它跳过 cargo;改了 Rust 要全量,否则二进制(连带 `bindings.ts`)是旧的
  —— 旧二进制会以同样的 “missing required key” 报错伪装成前端 bug。
