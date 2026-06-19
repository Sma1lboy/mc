//! 各整合包 / 实例格式的**字段级** serde 结构。
//!
//! 每个子模块对应一种格式,提供可直接 (反)序列化的类型(配 `docs/modules/modpack-formats.md`
//! 的字段级 schema)。这些结构是上层 importer `plan()` 的输入边界;统一产物(`ImportPlan`)
//! 与 importer 引擎在别处。
//!
//! 注意各格式的渠道差异:
//! - mrpack / curseforge / mcbbs:本地 zip,有 manifest 文件。
//! - multimc:本地目录(`mmc-pack.json` + `instance.cfg`)。
//! - packwiz:TOML 树(非 zip,常远程);本 crate 不引入 `toml` 依赖,自带最小读取器。
//! - atlauncher / technic:远程平台 API,本模块仅建模其响应结构。

pub mod atlauncher;
pub mod curseforge;
pub mod mcbbs;
pub mod mrpack;
pub mod multimc;
pub mod packwiz;
pub mod technic;
