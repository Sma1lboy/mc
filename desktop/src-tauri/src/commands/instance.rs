use super::*;

// --- read-only queries ----------------------------------------------------

#[tauri::command]
#[specta::specta]
pub fn list_roots() -> CmdResult<Vec<GameRoot>> {
    Ok(paths::discover_roots(&exe_dir(), &data_dir(), &custom_roots()))
}

// `async` so Tauri runs this on its async runtime rather than the main (UI)
// thread: scanning every instance (read each version json + base64-encode each
// icon) is heavy file I/O and would otherwise freeze the UI ("卡一下"), worst on
// roots holding big foreign instances. Same reasoning for the other read
// commands below marked async.
#[tauri::command]
#[specta::specta]
pub async fn list_instances(root: String) -> CmdResult<Vec<InstanceSummary>> {
    Ok(mc_core::instance::list_instances(&root_paths(&root)))
}

/// 取实例的游戏目录绝对路径(供「打开游戏目录」用前端 shell.open 打开)。
#[tauri::command]
#[specta::specta]
pub fn instance_dir(root: String, id: String) -> CmdResult<String> {
    let paths = root_paths(&root);
    let dir = Instance::new(&id, paths.root().to_path_buf()).dir();
    Ok(dir.to_string_lossy().to_string())
}

/// 用系统文件管理器打开一个路径(目录/文件)。直接调 OS,绕开 shell 插件只放行 URL 的作用域。
#[tauri::command]
#[specta::specta]
pub fn reveal_path(path: String) -> CmdResult<()> {
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(target_os = "windows")]
    let spawned = std::process::Command::new("explorer").arg(&path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(&path).spawn();
    spawned.map(|_| ()).map_err(err)
}

/// 取实例某个子目录的绝对路径并确保其存在(供「打开目录」用前端 shell.open 打开)。
/// sub = "mods" / "resourcepacks" / "shaderpacks" / "datapacks" / "saves" / "screenshots" / "config"。
#[tauri::command]
#[specta::specta]
pub fn instance_subdir(root: String, id: String, sub: String) -> CmdResult<String> {
    let inst = instance_of(&root, &id);
    let dir = match sub.as_str() {
        "mods" => inst.mods_dir(),
        "resourcepacks" => inst.resourcepacks_dir(),
        "shaderpacks" => inst.shaderpacks_dir(),
        "datapacks" => inst.datapacks_dir(),
        "saves" => inst.saves_dir(),
        "screenshots" => inst.screenshots_dir(),
        "config" => inst.game_dir().join("config"),
        other => return Err(format!("未知子目录: {other}")),
    };
    // 目录可能尚未被游戏创建过;先建好,避免 shell.open 打开不存在的路径失败。
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {e}"))?;
    Ok(dir.to_string_lossy().into_owned())
}

/// 删除一个实例。复用 mc-core 的 lifecycle::delete_instance:优先移入系统回收站
/// (可恢复),无 GUI 时回退永久删除;目录不存在视为已删除(幂等)。前端须先确认。
#[tauri::command]
#[specta::specta]
pub fn delete_instance(root: String, id: String) -> CmdResult<()> {
    mc_core::instance::lifecycle::delete_instance(&root_paths(&root), &id).map_err(err)
}

/// 复制一个实例:整目录复制 src_id → 新实例(id 由 new_name 唯一化),并把新实例
/// 的 instance.json name 设为 new_name。返回新实例 id。
#[tauri::command]
#[specta::specta]
pub fn copy_instance(root: String, src_id: String, new_name: String) -> CmdResult<String> {
    mc_core::instance::lifecycle::copy_instance_named(&root_paths(&root), &src_id, &new_name)
        .map_err(err)
}

/// 从零创建实例:装核心(原版或 + loader)→ 写命名实例。进度走 install://progress。
/// loader = "vanilla" / "fabric" / "quilt" / "forge" / "neoforge";forge/neoforge 需 loader_version。
#[tauri::command]
#[specta::specta]
pub async fn create_instance(
    app: AppHandle,
    root: String,
    name: String,
    mc_version: String,
    loader: String,
    loader_version: Option<String>,
) -> CmdResult<String> {
    let paths = root_paths(&root);
    let dl = make_downloader()?;
    let loader_opt = match parse_loader_kind(&loader) {
        None | Some(mc_core::types::LoaderKind::Vanilla) => None,
        Some(kind) => Some((kind, loader_version.unwrap_or_default())),
    };
    let tx = progress_channel(app, "install://progress", "准备");
    // 新实例的默认内存/Java 取自全局设置,让设置页的「默认内存 / Java 路径」真正生效。
    let g = settings_global();
    mc_core::instance::lifecycle::create_instance(
        &dl,
        &paths,
        &name,
        &mc_version,
        loader_opt,
        g.default_memory_mb,
        g.java_path.clone(),
        Some(tx),
    )
    .await
    .map_err(err)
}

/// 给一个已存在的实例添加 / 切换 mod 加载器(core)。进度走 install://progress。
/// loader = "fabric" / "quilt" / "forge" / "neoforge"(拒绝 vanilla / 无效值);
/// forge/neoforge 需 loader_version。返回之后应使用的实例 id —— 多数情况与传入 id
/// 相同,但「实例目录本身就是裸原版」的退化情形会返回一个新 id(避免自环)。
#[tauri::command]
#[specta::specta]
pub async fn install_loader(
    app: AppHandle,
    root: String,
    id: String,
    mc_version: String,
    loader: String,
    loader_version: Option<String>,
) -> CmdResult<String> {
    let paths = root_paths(&root);
    let dl = make_downloader()?;
    let kind = match parse_loader_kind(&loader) {
        Some(mc_core::types::LoaderKind::Vanilla) | None => {
            return Err(format!("无效的加载器: {loader}"));
        }
        Some(kind) => kind,
    };
    let tx = progress_channel(app, "install://progress", "准备");
    mc_core::instance::lifecycle::add_loader(
        &dl,
        &paths,
        &id,
        &mc_version,
        (kind, loader_version.unwrap_or_default()),
        Some(tx),
    )
    .await
    .map_err(err)
}

/// 列出某 loader 在指定 MC 版本下的可用构建号(新建实例的版本选择器用)。
/// loader = "forge" / "neoforge" / "fabric" / "quilt";其它(vanilla 等)返回空。
/// 返回值按「新→旧」排序,前端默认选第一个。网络/解析失败时由前端回退到手填输入框。
#[tauri::command]
#[specta::specta]
pub async fn list_loader_versions(
    loader: String,
    mc_version: String,
) -> CmdResult<Vec<String>> {
    let kind = match parse_loader_kind(&loader) {
        Some(k) => k,
        None => return Ok(Vec::new()),
    };
    let dl = make_downloader()?;
    mc_core::loader::list_loader_versions(&dl, kind, &mc_version)
        .await
        .map_err(err)
}

/// 读取某实例的配置(名字/内存/Java/JVM 参数/窗口…)。文件缺失返回默认值。
#[tauri::command]
#[specta::specta]
pub fn get_instance_config(root: String, id: String) -> CmdResult<mc_core::instance::InstanceConfig> {
    let inst = instance_of(&root, &id);
    inst.load_config().map_err(err)
}

/// 写某实例的配置(原子写入 instance.json)。
#[tauri::command]
#[specta::specta]
pub fn set_instance_config(
    root: String,
    id: String,
    config: mc_core::instance::InstanceConfig,
) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    inst.save_config(&config).map_err(err)
}

/// 本机内存信息(供 UI 在内存滑块旁展示「系统内存 X GB」)。
#[derive(Serialize, specta::Type)]
pub struct SystemMemory {
    /// 物理内存总量(MiB)。探测失败时为 0。
    pub total_mb: u64,
}

/// 读取本机物理内存总量(MiB)。纯探测,不读实例。
#[tauri::command]
#[specta::specta]
pub fn system_memory() -> CmdResult<SystemMemory> {
    Ok(SystemMemory { total_mb: mc_core::system::system_total_mem_mb() })
}

/// 为某实例推荐一个最大堆内存(MiB):综合本机物理内存与该实例已装 mod 数量。
/// 纯启发式(见 [`mc_core::system::suggest_memory_mb`]),按需读取一次 mod 列表 + 系统内存。
#[tauri::command]
#[specta::specta]
pub async fn suggest_instance_memory(root: String, id: String) -> CmdResult<u32> {
    let inst = instance_of(&root, &id);
    let mod_count = mc_core::instance::list_mods(&inst).len();
    let total = mc_core::system::system_total_mem_mb();
    Ok(mc_core::system::suggest_memory_mb(total, mod_count))
}

/// 设置某实例的标签:加载配置 → 规范化(去空白、去空项、去重、保序)→ 写回。
/// 自由格式标签,供库页分组 / 按标签筛选用。
#[tauri::command]
#[specta::specta]
pub fn set_instance_tags(root: String, id: String, tags: Vec<String>) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    let mut config = inst.load_config().map_err(err)?;
    let mut seen = std::collections::HashSet::new();
    config.tags = tags
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .collect();
    inst.save_config(&config).map_err(err)
}

/// 把任意图片设为实例图标(拷贝到 `versions/<id>/icon.png`)。source 为本地文件绝对路径。
/// 之后 list_instances 会把它探测为 data URL 回传前端。
#[tauri::command]
#[specta::specta]
pub fn set_instance_icon(root: String, id: String, source: String) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    inst.set_icon(std::path::Path::new(&source)).map_err(err)
}

/// 给整合包来源的实例补齐图标:实例还没有本地 `icon.png` 时,下载 `icon_url` 写入。
/// 早于「安装即存图标」特性安装的实例本地没图标,详情页发现缺失时调用一次,让侧栏/首页/详情
/// 统一显示整合包真实 logo,而不再退回默认像素占位。返回 `true` 表示这次补上了。
/// 已有图标或下载失败都安静返回 `false`(图标纯展示性,失败不打断任何流程)。
#[tauri::command]
#[specta::specta]
pub async fn backfill_instance_icon(root: String, id: String, icon_url: String) -> CmdResult<bool> {
    let inst = instance_of(&root, &id);
    if inst.has_icon() || icon_url.trim().is_empty() {
        return Ok(false);
    }
    let Ok(dl) = make_downloader() else { return Ok(false) };
    match dl.get_bytes(&icon_url).await {
        Ok(bytes) => Ok(inst.set_icon_bytes(&bytes).is_ok()),
        Err(_) => Ok(false),
    }
}

/// 列出某实例的截图(仅元数据,按修改时间倒序)。
#[tauri::command]
#[specta::specta]
pub async fn instance_screenshots(
    root: String,
    id: String,
) -> CmdResult<Vec<mc_core::instance::ScreenshotInfo>> {
    let inst = instance_of(&root, &id);
    Ok(mc_core::instance::list_screenshots(&inst))
}

/// 按需读取一张截图为 data URL(UI 滚动到哪张才取哪张)。
#[tauri::command]
#[specta::specta]
pub fn read_screenshot(root: String, id: String, file_name: String) -> CmdResult<String> {
    let inst = instance_of(&root, &id);
    mc_core::instance::read_screenshot(&inst, &file_name).map_err(err)
}

/// 删除一张截图(移入系统回收站,可找回)。
#[tauri::command]
#[specta::specta]
pub fn delete_screenshot(root: String, id: String, file_name: String) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    mc_core::instance::screenshots::delete_screenshot(&inst, &file_name).map_err(err)
}

/// 列出某实例的存档世界(名字/模式/大小/上次游玩…)。
#[tauri::command]
#[specta::specta]
pub async fn instance_worlds(root: String, id: String) -> CmdResult<Vec<mc_core::instance::WorldInfo>> {
    let inst = instance_of(&root, &id);
    Ok(mc_core::instance::list_worlds(&inst))
}

/// 列出某实例已保存的多人服务器(读 game_dir/servers.dat;文件不存在 → 空表)。
#[tauri::command]
#[specta::specta]
pub fn instance_servers(root: String, id: String) -> CmdResult<Vec<mc_core::instance::SavedServer>> {
    let inst = instance_of(&root, &id);
    mc_core::instance::read_servers(&inst.game_dir()).map_err(err)
}

/// 向某实例的 servers.dat 追加一条多人服务器(name 可空,address 必填)。
#[tauri::command]
#[specta::specta]
pub fn add_instance_server(root: String, id: String, name: String, address: String) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    mc_core::instance::add_server(&inst.game_dir(), &name, &address).map_err(err)
}

/// Ping 一个 Minecraft 服务器,返回在线/人数/延迟/MOTD(失败时返回离线状态,从不报错)。
#[tauri::command]
#[specta::specta]
pub async fn ping_server(address: String) -> CmdResult<mc_core::server_ping::ServerStatus> {
    Ok(mc_core::server_ping::ping_server(&address).await)
}

/// 删除一个存档世界(移入系统回收站,可找回)。
#[tauri::command]
#[specta::specta]
pub fn delete_world(root: String, id: String, folder: String) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    mc_core::instance::world::delete_world(&inst, &folder).map_err(err)
}

/// 把一个存档打成 zip 备份到 dest_path(完整 .zip 文件路径,由 UI 的另存为对话框给出),
/// 返回写出的 zip 绝对路径。
#[tauri::command]
#[specta::specta]
pub fn backup_world(root: String, id: String, folder: String, dest_path: String) -> CmdResult<String> {
    let inst = instance_of(&root, &id);
    mc_core::instance::world::backup_world(&inst, &folder, std::path::Path::new(&dest_path))
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(err)
}

/// 从一个 .zip 导入世界到实例 saves/,返回新世界文件夹名。zip 内需含 level.dat。
#[tauri::command]
#[specta::specta]
pub fn import_world_zip(root: String, id: String, path: String) -> CmdResult<String> {
    let inst = instance_of(&root, &id);
    mc_core::instance::import_world_zip(&inst, std::path::Path::new(&path)).map_err(err)
}

/// 重命名存档的显示名(改 level.dat 的 LevelName,不改文件夹名)。
#[tauri::command]
#[specta::specta]
pub fn rename_world(root: String, id: String, folder: String, new_name: String) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    mc_core::instance::world::rename_world(&inst, &folder, &new_name).map_err(err)
}

