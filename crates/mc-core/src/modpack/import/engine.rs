//! 共享执行引擎:所有格式复用的「取包 → 探测分发 → 解压子树 → plan → resolve → 建实例 →
//! 装 loader → 下文件 → 铺 overrides → 写溯源」管线(format-independent)。
//!
//! 等价 Prism 的 `InstanceCreationTask::executeTask` + `InstanceImportTask`,但适配本启动器
//! 「version == instance」的 `versions/<id>/` 目录模型。新增格式只写一个
//! [`ModpackImporter`] 并在 [`ImportEngine::with_defaults`] 里加一行;引擎零改动。
//!
//! 多源故障转移 + sha512 强校验在此**集中**实现一次(经 [`Downloader`] 的多候选下载),
//! 而非像 Prism 那样每文件 lambda 散落。

use std::path::PathBuf;

use mc_types::Progress;
use tokio::sync::watch;

use crate::download::{DownloadItem, Downloader};
use crate::error::{CoreError, Result};
use crate::instance::Instance;
use crate::modplatform::provider::ProviderRegistry;
use crate::paths::{ensure_dir, GamePaths};

use super::archive::{overlay_dir_safe, StagingDir, ZipArchiveIndex};
use super::{ArchiveIndex, BlockedFile, DetectMatch, ImportPlan, ManagedPack, ModpackImporter};

/// 导入来源:本地归档文件,或先下载后导入的远程 URL。
#[derive(Debug, Clone)]
pub enum ImportSource {
    /// 本地 `.zip`/`.mrpack` 等归档文件。
    LocalFile(PathBuf),
    /// 远程整包 URL —— 引擎先用 [`Downloader`] 下到临时文件,再当作本地归档导入。
    Url(String),
}

/// 导入选项。`instance_id`/`managed` 对应 Prism dispatcher 的 `extra_info`(provider 发起的
/// 安装 vs 裸 zip 拖入,以及就地更新已存在实例)。
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// 目标 game root(实例建在 `dest_root/versions/<id>/`)。
    pub dest_root: PathBuf,
    /// 指定实例 id;`None` 时由整合包名派生唯一 id。
    pub instance_id: Option<String>,
    /// 可选实例图标源文件(拷到实例目录 `icon.png`)。
    pub icon: Option<PathBuf>,
    /// 覆盖 plan 自带的溯源(provider 发起安装时传入更精确的来源)。
    pub managed: Option<ManagedPack>,
}

impl ImportOptions {
    /// 仅指定目标 root 的最小选项。
    pub fn new(dest_root: impl Into<PathBuf>) -> Self {
        ImportOptions { dest_root: dest_root.into(), instance_id: None, icon: None, managed: None }
    }
}

/// 导入结果。交互(可选 mod / blocked mod)在本启动器里**退化为回传数据**:逻辑留核心,
/// UI 薄(对齐 Prism `OptionalModDialog`/`BlockedModsDialog` 的数据化)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOutcome {
    /// 建好的实例 id。
    pub instance_id: String,
    /// 无第三方链接、需用户手动下载的文件(CF blocked);引擎已跳过它们。
    pub blocked: Vec<BlockedFile>,
    /// 被跳过的可选文件(下载失败但非必备)的 rel_path。
    pub skipped_optional: Vec<String>,
}

/// 共享导入引擎:一个下载器 + 一个 provider 注册表 + 按注册序排列的 importer 列表。
pub struct ImportEngine {
    dl: Downloader,
    registry: ProviderRegistry,
    importers: Vec<Box<dyn ModpackImporter>>,
}

impl ImportEngine {
    /// 用给定下载器与注册表建一个空引擎(无 importer);通常用 [`Self::with_defaults`]。
    pub fn new(dl: Downloader, registry: ProviderRegistry) -> Self {
        ImportEngine { dl, registry, importers: Vec::new() }
    }

    /// 注册全部内建 importer,**顺序即优先级**(对齐 `docs/modules/modpack-import.md` §4 的
    /// 判别优先级表):
    ///
    /// 1. mcbbs(`mcbbs.packmeta` —— 唯一命名标记,**先于** `manifest.json`)
    /// 2. multimc(`mmc-pack.json` / `instance.cfg` —— 目录即包,可嵌套一层)
    /// 3. modrinth(`modrinth.index.json`)
    /// 4. curseforge(`manifest.json` 且**无** `addons` —— 与 mcbbs 同名,靠内容判别;
    ///    `manifest.json` 会出现在 `overrides/` 内,故置于最后)
    ///
    /// mcbbs 早于 curseforge,且二者都看 `manifest.json` 时靠 `detect()` 内容判别区分
    /// (mcbbs 命中需 `addons`/`launchInfo`,curseforge 命中需**无**它们),互斥不冲突。
    pub fn with_defaults(dl: Downloader, registry: ProviderRegistry) -> Self {
        let mut engine = Self::new(dl, registry);
        engine.register(Box::new(super::mcbbs::McbbsImporter));
        engine.register(Box::new(super::multimc::MultiMcImporter));
        engine.register(Box::new(super::modrinth::ModrinthImporter));
        engine.register(Box::new(super::curseforge::CurseForgeImporter));
        engine
    }

    /// 追加一个 importer(注册序靠后 = 优先级更低)。
    pub fn register(&mut self, importer: Box<dyn ModpackImporter>) {
        self.importers.push(importer);
    }

    /// 注册的 importer 数量(测试 / 诊断用)。
    pub fn importer_count(&self) -> usize {
        self.importers.len()
    }

    /// **纯分发**:按注册顺序跑各 `detect()`,取**最高 confidence** 的命中;平局按注册序
    /// (先注册者胜)。返回 `(importer 下标, 命中)`,全不中返回 `None`。可单独单测。
    pub fn dispatch(&self, archive: &dyn ArchiveIndex) -> Option<(usize, DetectMatch)> {
        let mut best: Option<(usize, DetectMatch)> = None;
        for (i, importer) in self.importers.iter().enumerate() {
            if let Some(m) = importer.detect(archive) {
                let better = match &best {
                    // 严格大于才替换 → 平局保留先注册者。
                    Some((_, b)) => m.confidence > b.confidence,
                    None => true,
                };
                if better {
                    best = Some((i, m));
                }
            }
        }
        best
    }

    /// 执行一次完整导入。见模块文档的 10 步管线。
    pub async fn import(&self, src: ImportSource, opts: ImportOptions) -> Result<ImportOutcome> {
        self.import_with_progress(src, opts, None).await
    }

    /// 同 [`import`](Self::import),但把各阶段进度发到 `progress`(下载整合包 / 装核心 /
    /// 下载文件),供 UI 显示进度条 —— 整包下载常达数 GB,没有进度会像卡死。
    pub async fn import_with_progress(
        &self,
        src: ImportSource,
        opts: ImportOptions,
        progress: Option<watch::Sender<Progress>>,
    ) -> Result<ImportOutcome> {
        // ---- 1) 取归档:URL 先下到临时文件,本地文件直接用 ----
        // _tmp 持有临时下载文件的所有权,确保其生命周期覆盖整个导入。
        if let Some(tx) = &progress {
            let _ = tx.send(Progress::new("读取整合包"));
        }
        let (archive_path, _tmp) = self.acquire_archive(&src).await?;

        // ---- 2) 打开 zip 一次 → 建 ArchiveIndex → dispatch ----
        let raw_index = ZipArchiveIndex::open(&archive_path)?;
        // detect 的内容判别(CF vs MCBBS)需读 manifest.json;预取它喂给带缓存的只读视图。
        let prepared = raw_index.into_prepared(&["manifest.json", "mcbbs.packmeta"]);
        let (idx, det) = self
            .dispatch(&prepared)
            .ok_or_else(|| CoreError::other("无法识别的整合包格式(请确认是受支持的 .mrpack/zip)"))?;

        // ---- 3) 按 archive_root 把对应子树解压到 staging ----
        let staging = StagingDir::new()?;
        let mut archive = prepared.into_inner();
        archive.extract_subtree(&det.archive_root, staging.path())?;

        // ---- 4) importer.plan(staging) → ImportPlan ----
        let mut plan = self.importers[idx].plan(staging.path(), &det)?;

        // ---- 5) importer.resolve(dl, registry, &mut plan) → 填 unresolved + 收 blocked ----
        let blocked = self.importers[idx]
            .resolve(&self.dl, &self.registry, &mut plan)
            .await?;

        // ---- 6) 建实例目录 versions/<id>/ + 写 instance.json(name/内存)+ 溯源 ----
        let paths = GamePaths::new(opts.dest_root.clone());
        let instance_id = self.choose_instance_id(&paths, &opts, &plan);
        let inst = Instance::new(instance_id.clone(), paths.root().to_path_buf());
        let game_dir = inst.game_dir();
        // 事务性:记录本次是否「新建」了这个版本目录。若是且后续装核心/下载中途失败,
        // 回滚删除它 —— 否则会在库里留下一个可启动却残缺、与正常实例无法区分的坏实例。
        let created_dir = !paths.version_dir(&instance_id).exists();
        ensure_dir(&game_dir)?;

        let outcome = self
            .finish_import(&paths, &inst, &instance_id, &game_dir, &plan, &opts, &staging, blocked, progress)
            .await;

        match outcome {
            Ok(o) => Ok(o),
            Err(e) => {
                if created_dir {
                    let _ = std::fs::remove_dir_all(paths.version_dir(&instance_id));
                }
                Err(e)
            }
        }
    }

    /// 步骤 7–9:写配置/图标 → 装核心 + 溯源 → 下文件 → 铺 overrides。抽出来便于上层在
    /// 任一步失败时统一回滚已新建的实例目录(见 [`import`](Self::import))。
    #[allow(clippy::too_many_arguments)]
    async fn finish_import(
        &self,
        paths: &GamePaths,
        inst: &Instance,
        instance_id: &str,
        game_dir: &std::path::Path,
        plan: &ImportPlan,
        opts: &ImportOptions,
        staging: &StagingDir,
        blocked: Vec<BlockedFile>,
        progress: Option<watch::Sender<Progress>>,
    ) -> Result<ImportOutcome> {
        self.write_instance_config(inst, plan, opts)?;
        if let Some(icon_src) = &opts.icon {
            // 图标拷到实例目录 icon.png(失败不致命,仅记录)。
            if let Err(e) = std::fs::copy(icon_src, game_dir.join("icon.png")) {
                tracing::warn!(error = %e, "拷贝实例图标失败");
            }
        }

        // ---- 7) 装核心:loader(或原版)→ 得到实例应继承的版本 id ----
        if let Some(tx) = &progress {
            let _ = tx.send(Progress::new("安装核心 / 加载器"));
        }
        let core_id = self.install_core(paths, plan, progress.clone()).await?;
        // 让整合包实例本身成为**可启动版本**:写一个 inheritsFrom 核心版本的最小 version json,
        // 使 versions/<instance_id>/ 既是版本定义(继承 loader → 原版)又是游戏目录(mods 在此)。
        // 仅当实例 id 与核心 id 不同时才写(原版包且 id==mc_version 时核心 json 已就位)。
        if core_id != instance_id {
            self.write_instance_version_json(paths, instance_id, &core_id)?;
        }

        // ---- 8) 下文件:PlannedFile → DownloadItem → 多源下载(并发 + 校验)----
        if let Some(tx) = &progress {
            let _ = tx.send(Progress::new("下载整合包文件"));
        }
        let skipped_optional = self.download_files(game_dir, plan, progress).await?;

        // ---- 9) 铺 overrides:逐个 override_root 从 staging 经 safe_join 拷进 game_dir ----
        for root in &plan.override_roots {
            // override 根是 staging 下的子目录(解压子树时已落地)。
            let src_root = staging.path().join(root);
            overlay_dir_safe(&src_root, game_dir)?;
        }

        Ok(ImportOutcome {
            instance_id: instance_id.to_string(),
            blocked,
            skipped_optional,
        })
    }

    // -----------------------------------------------------------------------
    // 步骤实现(私有)
    // -----------------------------------------------------------------------

    /// 取归档路径:本地直接返回;URL 下到临时文件并返回(连同守护其生命周期的 `StagingDir`)。
    async fn acquire_archive(
        &self,
        src: &ImportSource,
    ) -> Result<(PathBuf, Option<StagingDir>)> {
        match src {
            ImportSource::LocalFile(p) => {
                if !p.is_file() {
                    return Err(CoreError::other(format!("整合包文件不存在: {}", p.display())));
                }
                Ok((p.clone(), None))
            }
            ImportSource::Url(url) => {
                let staging = StagingDir::new()?;
                let dest = staging.path().join("pack.archive");
                self.dl
                    .download_one(&DownloadItem::new(url.clone(), dest.clone(), None, None))
                    .await?;
                Ok((dest, Some(staging)))
            }
        }
    }

    /// 决定实例 id:opts 指定 → 用之;否则由整合包名 sanitise 出一个目录内唯一的 id。
    fn choose_instance_id(
        &self,
        paths: &GamePaths,
        opts: &ImportOptions,
        plan: &ImportPlan,
    ) -> String {
        if let Some(id) = &opts.instance_id {
            return id.clone();
        }
        // 由整合包名派生唯一目录名(sanitise + 冲突追加 -2/-3…)。
        crate::fs::dir_name_from_string(&plan.pack_name, &paths.versions_dir())
    }

    /// 写实例 `instance.json`:整合包名 + 推荐内存。
    fn write_instance_config(
        &self,
        inst: &Instance,
        plan: &ImportPlan,
        opts: &ImportOptions,
    ) -> Result<()> {
        let mut config = inst.load_config().unwrap_or_default();
        if !plan.pack_name.is_empty() {
            config.name = Some(plan.pack_name.clone());
        }
        if let Some(ram) = plan.recommended_ram_mib {
            // 夹到一个合理范围,避免 manifest 给出离谱值。
            config.memory_mb = ram.clamp(512, 65536) as u32;
        }
        // 整合包声明的附加启动参数(目前仅 MCBBS launchInfo);追加到实例配置。
        // **不**接受 manifest 里的 JavaPath / 启动命令(会执行任意二进制,见模块文档安全清单)。
        if !plan.extra_jvm_args.is_empty() {
            config.jvm_args = plan.extra_jvm_args.clone();
        }
        if !plan.extra_game_args.is_empty() {
            config.game_args = plan.extra_game_args.clone();
        }
        // 整合包来源溯源:持久化到实例配置,供 UI 展示来源 / 日后「更新整合包」。
        // 仅采用 `opts.managed`(由 provider 发起安装时带入的**确切**项目 id);裸 URL/zip
        // 导入不设它 → source 留 None,不拿 manifest 里的包名当作 id 伪造来源。
        if let Some(m) = &opts.managed {
            config.source = Some(crate::instance::InstanceSource {
                provider: m.platform.clone(),
                project_id: m.project_id.clone(),
                version_id: m.version_id.clone(),
            });
        }
        inst.save_config(&config)
    }

    /// 装核心:按 `plan.loader` 调现有 loader 安装器;原版调 `launch::install_version`。
    /// **返回实例应继承(`inheritsFrom`)的版本 id**:有 loader 时为 loader 版本 id,
    /// 原版时为 `mc_version`。
    ///
    /// 各 loader 安装器内部都会「缺原版则先装原版」,故原版安装只在无 loader 时显式触发。
    /// 注:Forge/NeoForge 使用 manifest 钉死的版本;Fabric/Quilt 现取最新稳定 loader
    /// (loader 向后兼容,通常无碍),钉版安装是后续增强。
    async fn install_core(
        &self,
        paths: &GamePaths,
        plan: &ImportPlan,
        progress: Option<watch::Sender<Progress>>,
    ) -> Result<String> {
        // 与「从零建实例」共用同一条装核心路径(见 loader::install_core)。
        crate::loader::install_core(&self.dl, paths, &plan.mc_version, plan.loader.as_ref(), progress)
            .await
    }

    /// 为整合包实例写一个最小 version json(`{id, inheritsFrom: core_id}`),使其成为
    /// **可启动版本**:启动时解析 inheritsFrom 链(实例 → loader → 原版)合成 profile,
    /// 而 mods/config 等位于 `versions/<instance_id>/`(= game_dir)。
    fn write_instance_version_json(
        &self,
        paths: &GamePaths,
        instance_id: &str,
        core_id: &str,
    ) -> Result<()> {
        let json = serde_json::json!({ "id": instance_id, "inheritsFrom": core_id });
        let raw = serde_json::to_string_pretty(&json)
            .map_err(|e| CoreError::Parse { what: "instance version json".into(), source: e })?;
        crate::fs::write_atomic(&paths.version_json(instance_id), raw.as_bytes())
    }

    /// 下载 `plan.files` 到 game_dir;**必备文件**任一失败即整体失败,**可选文件**失败仅记录
    /// 到 `skipped_optional`(尽力而为)。多源故障转移由 [`Downloader`] 处理。
    async fn download_files(
        &self,
        game_dir: &std::path::Path,
        plan: &ImportPlan,
        progress: Option<watch::Sender<Progress>>,
    ) -> Result<Vec<String>> {
        let mut required_items: Vec<DownloadItem> = Vec::new();
        let mut optional_items: Vec<(String, DownloadItem)> = Vec::new();

        for f in &plan.files {
            if f.sources.is_empty() {
                continue;
            }
            // zip-slip 防护:rel_path 必须收口在 game_dir 内。
            let Some(dest) = crate::fs::safe_join(game_dir, &f.rel_path) else {
                return Err(CoreError::other(format!("非法的整合包文件路径(越权): {}", f.rel_path)));
            };
            let item = DownloadItem {
                url: f.sources[0].clone(),
                mirrors: f.sources.iter().skip(1).cloned().collect(),
                path: dest,
                sha1: f.sha1.clone(),
                sha512: f.sha512.clone(),
                size: f.size,
                ..Default::default()
            };
            if f.required {
                required_items.push(item);
            } else {
                optional_items.push((f.rel_path.clone(), item));
            }
        }

        // 必备文件:必须全部到位。进度走必备文件下载(占整包耗时大头)。
        if !required_items.is_empty() {
            self.dl.download_all(required_items, progress).await?;
        }

        // 可选文件:尽力而为,失败仅记录被跳过的 rel_path。
        let mut skipped: Vec<String> = Vec::new();
        if !optional_items.is_empty() {
            let (rels, items): (Vec<String>, Vec<DownloadItem>) =
                optional_items.into_iter().unzip();
            let outcome = self.dl.download_batch(items, None).await?;
            for (item, _err) in &outcome.failed {
                // 用落盘路径回找对应 rel(失败项保留了 item)。
                if let Some(rel) = rels.iter().find(|r| {
                    crate::fs::safe_join(game_dir, r).as_deref() == Some(item.path.as_path())
                }) {
                    skipped.push(rel.clone());
                }
            }
        }
        Ok(skipped)
    }
}
