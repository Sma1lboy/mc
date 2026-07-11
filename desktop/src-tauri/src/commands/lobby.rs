use super::*;

// ============================================================================
// 联机大厅 P2 —— 拉起 / 停止 / 查询某领域的 EasyTier 虚拟局域网会话。
//
// 纯逻辑(参数构造 / 节点挑选 / peer 表解析)都在 `mc_core::lobby`,本层只做三件「壳」的事:
// 解析二进制、按平台**提权**拉起 `easytier-core`(建 TUN 需要 root/管理员),以及调用
// `easytier-cli peer` 取状态。二进制缺失时返回清晰的「请安装 EasyTier」错误,绝不 panic。
// ============================================================================

/// 已拉起的 EasyTier 会话句柄。Linux 直接持有特权子进程(kill 即停);macOS 经 osascript
/// 让 `easytier-core` 脱离我们后台运行(GUI 管理员授权),只记 pid + pidfile,停止时再用
/// osascript 提权 `kill`。Windows 等暂未支持(空枚举,`LobbyState` 永远为 `None`)。
enum LobbyProc {
    /// 直接持有的子进程(Linux 经 `pkexec` 提权,或任一平台用免密特权核心直接拉起)。
    #[cfg(unix)]
    Child(std::process::Child),
    #[cfg(target_os = "macos")]
    DetachedPid { pid: u32, pidfile: PathBuf },
}

impl LobbyProc {
    /// 终止会话。容错:已退出 / kill 失败都不致命(stop 语义是「确保停了」)。
    fn kill(self) {
        match self {
            #[cfg(unix)]
            LobbyProc::Child(mut c) => {
                let _ = c.kill();
                let _ = c.wait();
            }
            #[cfg(target_os = "macos")]
            LobbyProc::DetachedPid { pid, pidfile } => {
                let script =
                    format!("do shell script \"kill {pid}\" with administrator privileges");
                let _ = std::process::Command::new("osascript").arg("-e").arg(&script).output();
                let _ = std::fs::remove_file(&pidfile);
            }
        }
    }
}

/// 进程级会话状态:同一时刻最多一个 EasyTier 会话。`.manage()` 进 Tauri 状态(见 lib.rs)。
#[derive(Default)]
pub struct LobbyState {
    inner: Mutex<Option<LobbyProc>>,
}

/// 解析 EasyTier 二进制(`easytier-core` / `easytier-cli`)。依次找:① 与本程序同级(打包随附)
/// 或同级 `easytier/` 子目录(及 macOS `.app` 的 Resources);② `PATH`;③ 常见安装目录(GUI
/// 启动的应用常只继承精简 PATH,Homebrew/`/usr/local/bin` 可能不在内)。找不到 → `None`。
fn easytier_bin(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let mut roots: Vec<PathBuf> = vec![exe_dir(), exe_dir().join("easytier")];
    #[cfg(target_os = "macos")]
    roots.push(exe_dir().join("../Resources/easytier"));
    if let Some(p) = std::env::var_os("PATH") {
        roots.extend(std::env::split_paths(&p));
    }
    #[cfg(unix)]
    roots.extend(
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/usr/local/sbin"]
            .iter()
            .map(PathBuf::from),
    );
    roots.into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

/// EasyTier 缺失时给用户的清晰指引(后端错误串沿用项目既有的中文文案约定)。
fn easytier_missing_err() -> String {
    "未找到 EasyTier(easytier-core / easytier-cli)。请先安装 EasyTier 并确保它在 PATH 中,然后重试。下载:https://easytier.cn".to_string()
}

/// shell 单引号转义:把字符串包进 `'...'`,内部单引号写成 `'\''`。
#[cfg(unix)]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 按平台提权拉起 `easytier-core`(建 TUN 必须 root/管理员)。
#[cfg(target_os = "macos")]
fn spawn_elevated(
    core: &Path,
    args: &[String],
    pidfile: &Path,
    logfile: &Path,
) -> CmdResult<LobbyProc> {
    // 组一条 shell 命令:后台跑 core(输出进 logfile),前台把它的 pid 写进 pidfile。
    let mut shell = sh_quote(&core.to_string_lossy());
    for a in args {
        shell.push(' ');
        shell.push_str(&sh_quote(a));
    }
    shell.push_str(&format!(
        " >{} 2>&1 & echo $! >{}",
        sh_quote(&logfile.to_string_lossy()),
        sh_quote(&pidfile.to_string_lossy())
    ));
    // 再把整条命令转义进 AppleScript 字符串字面量(反斜杠、双引号)。
    let esc = shell.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("do shell script \"{esc}\" with administrator privileges");
    let out = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(err)?;
    if !out.status.success() {
        return Err("已取消授权或开启失败:开启联机需要管理员权限。".to_string());
    }
    // do shell script 返回前已同步写好 pidfile(`& echo $!` 在前台)。
    let pid = std::fs::read_to_string(pidfile)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .ok_or_else(|| "无法确定 easytier-core 进程 pid".to_string())?;
    Ok(LobbyProc::DetachedPid { pid, pidfile: pidfile.to_path_buf() })
}

/// Linux:Polkit(`pkexec`)提权,直接持有子进程。
#[cfg(target_os = "linux")]
fn spawn_elevated(
    core: &Path,
    args: &[String],
    _pidfile: &Path,
    _logfile: &Path,
) -> CmdResult<LobbyProc> {
    let child = std::process::Command::new("pkexec")
        .arg(core)
        .args(args)
        .spawn()
        .map_err(|e| format!("提权拉起 easytier-core 失败(需要 pkexec/Polkit):{e}"))?;
    Ok(LobbyProc::Child(child))
}

/// Windows 等:暂未实现一键提权(TODO:`Start-Process -Verb RunAs`)。
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn spawn_elevated(
    _core: &Path,
    _args: &[String],
    _pidfile: &Path,
    _logfile: &Path,
) -> CmdResult<LobbyProc> {
    Err("当前系统暂不支持一键开启联机(Windows 支持开发中)。".to_string())
}

/// 开启某领域的 EasyTier 联机会话:取凭据 → 挑节点 → 构参 → **提权**拉起 easytier-core。
/// 幂等:先停掉任何旧会话再起新的。UI 随后轮询 [`lobby_status`]。
#[tauri::command]
#[specta::specta]
pub async fn lobby_start(
    client: State<'_, mc_core::server::ServerClient>,
    lobby: State<'_, LobbyState>,
    realm_id: String,
    mode: String,
) -> CmdResult<()> {
    // 幂等:先停旧会话。
    let prev = lobby.inner.lock().unwrap().take();
    if let Some(p) = prev {
        p.kill();
    }

    let creds = client.realm_lobby(&realm_id).await.map_err(err)?;
    let node = mc_core::lobby::pick_node(&creds, &mode)
        .ok_or_else(|| "联机大厅没有可用的会合节点。".to_string())?;
    // hostname:登录用户名 → 名称 → 兜底,标识本机在别人 peer 表里。
    let hostname = client
        .me()
        .await
        .ok()
        .and_then(|u| u.username.or(u.name))
        .unwrap_or_else(|| "kobe-peer".to_string());
    let args = mc_core::lobby::easytier_core_args(&creds, &node.addr, &hostname);

    let dir = data_dir().join("lobby");
    std::fs::create_dir_all(&dir).map_err(err)?;
    let pidfile = dir.join("easytier.pid");
    let logfile = dir.join("easytier.log");

    // 免密一键已就绪(root 拥有 + setuid 的特权核心)→ 直接拉起,**不弹**管理员授权;
    // 否则回退到每次开启都提权的方案(macOS osascript / Linux pkexec)。
    let proc = if let Some(priv_core) = privileged_core() {
        spawn_privileged_direct(&priv_core, &args, &logfile)?
    } else {
        let core = easytier_bin("easytier-core").ok_or_else(easytier_missing_err)?;
        spawn_elevated(&core, &args, &pidfile, &logfile)?
    };
    *lobby.inner.lock().unwrap() = Some(proc);
    tracing::info!(target: "daemon", "联机会话已开启(realm={realm_id}, mode={mode}, node={})", node.addr);
    Ok(())
}

/// 断开当前 EasyTier 会话(容错:已停止也返回 Ok)。
#[tauri::command]
#[specta::specta]
pub fn lobby_stop(lobby: State<'_, LobbyState>) -> CmdResult<()> {
    let proc = lobby.inner.lock().unwrap().take();
    if let Some(p) = proc {
        p.kill();
        tracing::info!(target: "daemon", "联机会话已断开");
    }
    Ok(())
}

/// 查询联机会话状态:无会话 → `running:false`;有会话则跑 `easytier-cli peer` 解析。
/// cli 偶发失败不报错(返回 `running:true` + 空 peers),避免刚起步时 UI 抖动。
#[tauri::command]
#[specta::specta]
pub fn lobby_status(lobby: State<'_, LobbyState>) -> CmdResult<mc_core::lobby::LobbyStatus> {
    let running = lobby.inner.lock().unwrap().is_some();
    if !running {
        return Ok(mc_core::lobby::LobbyStatus { running: false, virtual_ip: None, peers: vec![] });
    }
    let empty = || mc_core::lobby::LobbyStatus { running: true, virtual_ip: None, peers: vec![] };
    let Some(cli) = status_cli() else {
        return Ok(empty());
    };
    match std::process::Command::new(&cli).arg("peer").output() {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let peers = mc_core::lobby::parse_peer_table(&text);
            Ok(mc_core::lobby::status_from_peers(peers))
        }
        _ => Ok(empty()),
    }
}

// ----------------------------------------------------------------------------
// 联机大厅 —— 可选的「免密一键(一次性提权)」。
//
// 痛点:每次「开启联机」都弹一次管理员授权(建 TUN 需要 root)。这里提供**一次性**提权:
// 把随附的 `easytier-core` 拷进一个 **root 拥有的受保护目录**,owner 设为 root 且打上 setuid
// 位。之后开启联机便能 setuid-root 直接建 TUN,**不再弹授权**。
//
// 安全关键:**绝不**对「用户可写目录」里的二进制 setuid(那是本地提权漏洞 —— 任何进程都能改
// 写那个文件再以 root 跑)。因此目标目录固定在 root 才能写的 `/usr/local/libexec/kobemc`,
// 拷贝 + chown + chmod 全在同一次管理员授权的脚本里完成。
// ----------------------------------------------------------------------------

/// 免密特权核心安放的 root 拥有的受保护目录(macOS / Linux)。
#[cfg(unix)]
const PRIVILEGED_DIR: &str = "/usr/local/libexec/kobemc";

#[cfg(target_os = "macos")]
const ROOT_OWNER: &str = "root:wheel";
#[cfg(target_os = "linux")]
const ROOT_OWNER: &str = "root:root";

/// 已就绪的免密特权核心:存在 + owner 为 root(uid 0)+ 带 setuid 位(`mode & 0o4000`)。
/// 三者全满足才算「免密就绪」,据此 [`lobby_start`] 决定直接拉起还是回退提权。其他平台恒 `None`。
#[cfg(unix)]
fn privileged_core() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let p = PathBuf::from(PRIVILEGED_DIR).join("easytier-core");
    let md = std::fs::metadata(&p).ok()?;
    (md.uid() == 0 && (md.mode() & 0o4000) != 0).then_some(p)
}

#[cfg(not(unix))]
fn privileged_core() -> Option<PathBuf> {
    None
}

/// 取状态用的 `easytier-cli`:优先免密目录里的副本(若存在),否则随附 / PATH 里的那个。
/// 查询 peer 不需要 root,所以这里不校验 owner / setuid,存在即用。
fn status_cli() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        let p = PathBuf::from(PRIVILEGED_DIR).join("easytier-cli");
        if p.is_file() {
            return Some(p);
        }
    }
    easytier_bin("easytier-cli")
}

/// 用免密特权核心**直接**(无 osascript / pkexec)拉起 easytier-core,stdout/stderr 进日志,
/// 持有子进程句柄。setuid-root 让它有权建 TUN。
#[cfg(unix)]
fn spawn_privileged_direct(core: &Path, args: &[String], logfile: &Path) -> CmdResult<LobbyProc> {
    let log = std::fs::File::create(logfile).map_err(err)?;
    let log2 = log.try_clone().map_err(err)?;
    let child = std::process::Command::new(core)
        .args(args)
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log2))
        .spawn()
        .map_err(|e| format!("拉起免密 easytier-core 失败:{e}"))?;
    Ok(LobbyProc::Child(child))
}

#[cfg(not(unix))]
fn spawn_privileged_direct(_core: &Path, _args: &[String], _logfile: &Path) -> CmdResult<LobbyProc> {
    Err("当前系统不支持免密直拉。".to_string())
}

/// 组装「拷贝 + chown root + setuid」的 shell 脚本(在一次管理员授权里执行)。
#[cfg(unix)]
fn privileged_install_script(core: &Path, cli: Option<&Path>) -> String {
    let dir = PRIVILEGED_DIR;
    let core_dst = format!("{dir}/easytier-core");
    let mut parts = vec![
        format!("mkdir -p {}", sh_quote(dir)),
        format!("cp {} {}", sh_quote(&core.to_string_lossy()), sh_quote(&core_dst)),
    ];
    if let Some(cli) = cli {
        let cli_dst = format!("{dir}/easytier-cli");
        parts.push(format!("cp {} {}", sh_quote(&cli.to_string_lossy()), sh_quote(&cli_dst)));
        parts.push(format!("chown {} {}", ROOT_OWNER, sh_quote(&cli_dst)));
        parts.push(format!("chmod 0755 {}", sh_quote(&cli_dst)));
    }
    parts.push(format!("chown {} {}", ROOT_OWNER, sh_quote(&core_dst)));
    parts.push(format!("chmod 4755 {}", sh_quote(&core_dst)));
    parts.join(" && ")
}

/// macOS:一次 osascript 管理员授权,装好 root 拥有 + setuid 的特权核心。
#[cfg(target_os = "macos")]
fn setup_privileged_impl() -> CmdResult<bool> {
    let core = easytier_bin("easytier-core").ok_or_else(easytier_missing_err)?;
    let cli = easytier_bin("easytier-cli");
    let shell = privileged_install_script(&core, cli.as_deref());
    let esc = shell.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("do shell script \"{esc}\" with administrator privileges");
    let out =
        std::process::Command::new("osascript").arg("-e").arg(&script).output().map_err(err)?;
    if !out.status.success() {
        return Err("已取消授权或免密设置失败:需要管理员权限。".to_string());
    }
    Ok(privileged_core().is_some())
}

/// Linux:pkexec 一次授权,装好 root 拥有 + setuid 的特权核心(readiness 判定与 macOS 一致)。
#[cfg(target_os = "linux")]
fn setup_privileged_impl() -> CmdResult<bool> {
    let core = easytier_bin("easytier-core").ok_or_else(easytier_missing_err)?;
    let cli = easytier_bin("easytier-cli");
    let shell = privileged_install_script(&core, cli.as_deref());
    let out = std::process::Command::new("pkexec")
        .arg("/bin/sh")
        .arg("-c")
        .arg(&shell)
        .output()
        .map_err(|e| format!("提权失败(需要 pkexec/Polkit):{e}"))?;
    if !out.status.success() {
        return Err("已取消授权或免密设置失败:需要管理员权限。".to_string());
    }
    Ok(privileged_core().is_some())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn setup_privileged_impl() -> CmdResult<bool> {
    Ok(false)
}

/// 一次性提权:装一个 root 拥有 + setuid 的 `easytier-core` 副本,让之后的「开启联机」免授权。
/// 返回安装后是否确已免密就绪。Windows 等暂不支持(返回 `false`)。
#[tauri::command]
#[specta::specta]
pub fn lobby_setup_privileged() -> CmdResult<bool> {
    setup_privileged_impl()
}

/// 是否已「免密就绪」:特权核心存在 + owner 为 root + 带 setuid 位。
#[tauri::command]
#[specta::specta]
pub fn lobby_privileged_ready() -> CmdResult<bool> {
    Ok(privileged_core().is_some())
}

