use super::*;

// --- DTOs that differ from the core types --------------------------------

/// JavaInstall with the version flattened to a string (the core keeps it
/// structured; the UI only displays it).
#[derive(Serialize, specta::Type)]
pub struct JavaDto {
    pub path: String,
    pub version: String,
    pub is_64bit: bool,
    pub source: String,
}

#[tauri::command]
#[specta::specta]
pub async fn list_versions(snapshot: bool) -> CmdResult<Vec<ManifestVersion>> {
    let dl = make_downloader()?;
    let all = meta::fetch_manifest(&dl).await.map_err(err)?;
    Ok(if snapshot {
        all
    } else {
        all.into_iter()
            .filter(|v| matches!(v.kind, mc_core::types::ReleaseKind::Release))
            .collect()
    })
}
#[tauri::command]
#[specta::specta]
pub async fn detect_java() -> CmdResult<Vec<JavaDto>> {
    let installs = java::detect_all().await;
    Ok(installs
        .into_iter()
        .map(|j| JavaDto {
            path: j.path.to_string_lossy().into_owned(),
            version: j.version.to_string(),
            is_64bit: j.is_64bit,
            source: j.source,
        })
        .collect())
}

// --- progress / log plumbing ---------------------------------------------

#[tauri::command]
#[specta::specta]
pub async fn install_version(app: AppHandle, root: String, id: String) -> CmdResult<()> {
    let paths = root_paths(&root);
    let dl = make_downloader()?;
    let manifest = meta::fetch_manifest(&dl).await.map_err(err)?;
    let entry = manifest
        .into_iter()
        .find(|v| v.id == id)
        .ok_or_else(|| format!("版本 {id} 不在清单中"))?;

    let tx = progress_channel(app, "install://progress", "准备");
    launch::install_version(&dl, &paths, &entry, Some(tx))
        .await
        .map_err(err)
}

/// 运行中的游戏进程表:instance id → 给该进程 reaper 任务发「请停止」的一次性信号。
///
/// 进程自然退出时由 reaper 自己把条目移除;[`stop_instance`] 主动停止时把 sender 取出
/// 并发信号。用 `Arc` 包裹以便克隆进 `'static` 的后台任务里(自然退出后自我注销)。
#[derive(Clone, Default)]
pub struct RunningGames {
    inner: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

impl RunningGames {
    fn register(&self, id: String, kill: oneshot::Sender<()>) {
        self.inner.lock().unwrap().insert(id, kill);
    }
    fn unregister(&self, id: &str) {
        self.inner.lock().unwrap().remove(id);
    }
    pub(super) fn is_running(&self, id: &str) -> bool {
        self.inner.lock().unwrap().contains_key(id)
    }
    /// 取出并移除某实例的停止信号(若在运行)。
    fn take(&self, id: &str) -> Option<oneshot::Sender<()>> {
        self.inner.lock().unwrap().remove(id)
    }
    fn ids(&self) -> Vec<String> {
        self.inner.lock().unwrap().keys().cloned().collect()
    }
}

/// 启动一个实例。进程被登记进 [`RunningGames`];生命周期通过事件回传 UI:
/// 进度走 `launch://progress`,日志走 `game://log`,**真正 spawn 成功**后发
/// `game://started { id }`,进程退出后发 `game://exit { id, code, success, reason }`
/// (非零退出会跑崩溃诊断,把人话原因 + 建议一并带回)。
///
/// 同一实例已在运行时直接拒绝,避免重复开多个 JVM。
#[tauri::command]
#[specta::specta]
pub async fn launch_instance(
    app: AppHandle,
    state: State<'_, RunningGames>,
    root: String,
    id: String,
    name: String,
    online: bool,
    server: Option<String>,
) -> CmdResult<()> {
    if state.is_running(&id) {
        return Err(format!("实例「{id}」已经在运行了"));
    }

    let paths = root_paths(&root);
    let dl = make_downloader()?;

    // 选中的微软账号若(接近)过期,先用 refresh_token 免浏览器续期(best-effort:
    // 失败就用现有 token 继续启动,不阻断游戏)。
    let accounts_path = accounts_path();
    if let Ok(mut store) = AccountStore::load(&accounts_path) {
        let _ = auth::refresh_selected_microsoft(&mut store, &msa_client(), 600).await;
        // 外置登录账号:启动前校验 token,失效则用 client_token 免密续期并写回
        // (best-effort:校验/续期失败就用现有 token 继续,不阻断启动)。
        let _ = auth::refresh_selected_yggdrasil(&mut store, dl.client().clone()).await;
    }

    // 外置登录账号:启动前下载 authlib-injector,并把 `-javaagent` 注入 JVM 参数,
    // 否则游戏仍走 Mojang 认证、外置皮肤/联机校验都不生效。
    let mut extra_jvm_args: Vec<String> = Vec::new();
    if let Some(yg_base) = AccountStore::load(&accounts_path)
        .ok()
        .and_then(|s| s.selected_account().and_then(|a| a.yggdrasil_base.clone()))
    {
        match auth::yggdrasil::download_authlib_injector(&dl, &data_dir().join("authlib")).await {
            Ok(jar) => extra_jvm_args.push(auth::yggdrasil::javaagent_arg(&jar, &yg_base)),
            Err(e) => return Err(format!("下载 authlib-injector 失败:{e}")),
        }
    }

    // Prefer the selected stored account; fall back to an offline session.
    let session = AccountStore::load(&accounts_path)
        .ok()
        .and_then(|s| s.selected_session())
        .unwrap_or_else(|| auth::offline_session(&name));

    // 是否联网修复文件:选了正版账号就联网(启动前补齐/修复缺损文件),离线账号走纯离线。
    // 离线 session 由 auth::offline_session 固定写入 access_token = "0" 标识。UI 传入的
    // online 作为下限,这样三个入口(Home/Library/经典)行为一致,不再因为某个入口硬编码
    // online=false 而跳过文件修复、导致残缺实例启动后神秘崩溃。
    let is_offline = session.access_token == "0" || session.access_token.is_empty();
    let online = online || !is_offline;

    let spec = LaunchSpec {
        instance: Instance::new(&id, paths.root().to_path_buf()),
        session,
        java_path: None,
        launcher_name: LAUNCHER_NAME.to_string(),
        launcher_version: LAUNCHER_VERSION.to_string(),
        online,
        runtimes_dir: Some(data_dir().join("java")),
        global_java_path: settings_global().java_path.filter(|p| !p.is_empty()).map(PathBuf::from),
        extra_jvm_args,
        server_override: server,
        game_dir_override: None,
        natives_dir_override: None,
    };

    let tx = progress_channel(app.clone(), "launch://progress", "准备");

    let mut child = launch::launch(spec, &dl, Some(tx)).await.map_err(err)?;

    // 滚动保留最近若干行输出,供进程退出后的崩溃诊断使用(崩溃原因多在 stderr 末尾)。
    let log_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    // Stream the game's stdout/stderr as log events (also drains the pipes so the
    // child never blocks on a full buffer).
    use tokio::io::{AsyncBufReadExt, BufReader};
    if let Some(out) = child.stdout.take() {
        let app = app.clone();
        let tail = log_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                push_tail(&tail, &line);
                let _ = app.emit("game://log", GameLog { line, level: "info" });
            }
        });
    }
    if let Some(e) = child.stderr.take() {
        let app = app.clone();
        let tail = log_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(e).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                push_tail(&tail, &line);
                let _ = app.emit("game://log", GameLog { line, level: "error" });
            }
        });
    }

    // 登记进程 + 通知 UI「真的起来了」(成功提示应以此为准,而非第一行日志)。
    let (kill_tx, kill_rx) = oneshot::channel::<()>();
    state.register(id.clone(), kill_tx);
    let _ = app.emit("game://started", GameStarted { id: id.clone() });

    // 后台 reaper:等待自然退出或停止信号,reap 进程,注销登记,并回传退出/崩溃信息。
    let registry = state.inner_handle();
    tokio::spawn(async move {
        let status = tokio::select! {
            s = child.wait() => s.ok(),
            _ = kill_rx => {
                let _ = child.start_kill();
                child.wait().await.ok()
            }
        };
        registry.unregister(&id);

        let code = status.and_then(|s| s.code());
        let success = status.map(|s| s.success()).unwrap_or(false);
        // 异常退出时保留日志尾部,供前端崩溃面板折叠查看 + 「复制诊断」;正常退出留空。
        let tail = if success {
            String::new()
        } else {
            log_tail.lock().unwrap().join("\n")
        };
        let analysis = if success {
            None
        } else {
            mc_core::diagnostics::analyze_exit(code.unwrap_or(-1), &tail)
        };
        let (reason, suggestions, category, matched) = match analysis {
            Some(a) => (
                Some(a.reason),
                a.suggestions,
                Some(a.category.slug().to_string()),
                a.matched,
            ),
            None => (None, Vec::new(), None, None),
        };
        let _ = app.emit(
            "game://exit",
            GameExit {
                id,
                code,
                success,
                reason,
                suggestions,
                category,
                matched,
                log_tail: tail,
            },
        );
    });

    Ok(())
}

/// 停止一个正在运行的实例(向其 reaper 发停止信号;reaper 杀进程并广播 `game://exit`)。
/// 实例不在运行时为 no-op。
#[tauri::command]
#[specta::specta]
pub fn stop_instance(state: State<'_, RunningGames>, id: String) -> CmdResult<()> {
    if let Some(kill) = state.take(&id) {
        let _ = kill.send(());
    }
    Ok(())
}

/// 当前正在运行的实例 id 列表(供 UI 挂载时同步运行态)。
#[tauri::command]
#[specta::specta]
pub fn running_instances(state: State<'_, RunningGames>) -> CmdResult<Vec<String>> {
    Ok(state.ids())
}

impl RunningGames {
    /// 克隆出可移动进 `'static` 后台任务的句柄(共享同一张表)。
    fn inner_handle(&self) -> RunningGames {
        self.clone()
    }
}

/// 把一行输出追加进滚动日志尾部,封顶 400 行,避免长会话无限增长。
fn push_tail(tail: &Arc<Mutex<Vec<String>>>, line: &str) {
    let mut t = tail.lock().unwrap();
    t.push(line.to_string());
    if t.len() > 400 {
        let overflow = t.len() - 400;
        t.drain(0..overflow);
    }
}

#[derive(Serialize, Clone, specta::Type)]
pub struct GameLog {
    line: String,
    level: &'static str,
}

#[derive(Serialize, Clone, specta::Type)]
pub struct GameStarted {
    id: String,
}

#[derive(Serialize, Clone, specta::Type)]
pub struct GameExit {
    id: String,
    /// 进程退出码(被信号杀死时可能为 `None`)。
    code: Option<i32>,
    success: bool,
    /// 非零退出时的人话崩溃原因(诊断命中才有)。
    reason: Option<String>,
    /// 崩溃诊断给出的可执行建议(可能为空)。
    suggestions: Vec<String>,
    /// 崩溃类别 slug(前端据此本地化类别标签,如 `out_of_memory`);诊断命中才有。
    category: Option<String>,
    /// 命中的关键日志行(截断到 200 字符),作为崩溃证据展示。
    matched: Option<String>,
    /// 异常退出时保留的日志尾部(最近若干行,换行连接),供折叠查看与「复制诊断」;正常退出为空。
    log_tail: String,
}
