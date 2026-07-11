use super::*;

pub(crate) async fn cmd_register_account(email: &str, password: &str) -> Result<()> {
    let client = mc_core::server::ServerClient::new()?;
    println!("在 {} 注册账号 {email}…", client.base_url());
    // name 取邮箱本地部分。
    let name = email.split('@').next().unwrap_or(email);
    let user = client.register(email, password, name).await?;
    println!(
        "✓ 注册成功: id={} email={}",
        user.id,
        user.email.as_deref().unwrap_or("-")
    );
    // 同一个 client 保留了会话 cookie,get-session 无需再传 token。
    let me = client.me().await?;
    println!(
        "✓ 会话校验: id={} email={}",
        me.id,
        me.email.as_deref().unwrap_or("-")
    );
    Ok(())
}

pub(crate) async fn cmd_server_health() -> Result<()> {
    let client = mc_core::server::ServerClient::new()?;
    println!("ping {} …", client.base_url());
    let v = client.health().await?;
    println!("{v}");
    Ok(())
}

pub(crate) async fn cmd_launch(
    cli: &Cli,
    id: &str,
    name: &str,
    use_account: bool,
    online: bool,
    java_path: Option<PathBuf>,
) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    let accounts_path = accounts_path();

    // 选中的账号在启动前续期,镜像桌面端行为,避免 >24h 的旧 session 静默掉线:
    // 微软走 refresh_token 免浏览器续期;外置(Yggdrasil)走 validate/refresh。续期失败
    // best-effort 忽略,用现有 token 继续启动(由皮肤站/会话服务器在游戏内最终把关)。
    let mut extra_jvm_args: Vec<String> = Vec::new();
    let session = if use_account {
        let mut store = AccountStore::load(&accounts_path)?;
        let _ = auth::refresh_selected_microsoft(&mut store, &msa_client(), 600).await;
        if let Err(e) = refresh_selected_yggdrasil(&mut store).await {
            eprintln!("外置登录续期失败(用现有 token 继续):{e}");
        }
        // 外置账号:下载 authlib-injector 并注入 `-javaagent`,否则外置皮肤/联机校验不生效。
        if let Some(yg_base) =
            store.selected_account().and_then(|a| a.yggdrasil_base.clone())
        {
            let jar = auth::yggdrasil::download_authlib_injector(&dl, &data_dir().join("authlib"))
                .await
                .context("下载 authlib-injector")?;
            extra_jvm_args.push(auth::yggdrasil::javaagent_arg(&jar, &yg_base));
        }
        store
            .selected_session()
            .context("没有选中的账号,先运行 `mc login`")?
    } else {
        offline_session(name)
    };

    println!("启动 {} (玩家: {}) …", id, session.username);
    let spec = LaunchSpec {
        instance: Instance::new(id, paths.root().to_path_buf()),
        session,
        java_path,
        launcher_name: LAUNCHER_NAME.to_string(),
        launcher_version: LAUNCHER_VERSION.to_string(),
        online,
        runtimes_dir: Some(data_dir().join("java")),
        global_java_path: GlobalSettings::load(&data_dir())
            .unwrap_or_default()
            .java_path
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from),
        extra_jvm_args,
        server_override: None,
    };

    let (tx, mut rx) = tokio::sync::watch::channel(mc_core::types::Progress::new("准备"));
    let printer = tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let p = rx.borrow().clone();
            println!("  {} {}/{}", p.stage, p.current, p.total);
        }
    });

    let mut child = launch::launch(spec, &dl, Some(tx)).await?;
    let _ = printer.await;
    println!("✓ 游戏进程已启动 (pid {:?})。游戏日志:", child.id());

    // Drain the child's piped stdout/stderr so the game does not block on a full
    // pipe buffer, and surface the log so we can see it actually booting.
    use tokio::io::{AsyncBufReadExt, BufReader};
    if let Some(out) = child.stdout.take() {
        let mut lines = BufReader::new(out).lines();
        tokio::spawn(async move {
            while let Ok(Some(l)) = lines.next_line().await {
                println!("[game] {l}");
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let mut lines = BufReader::new(err).lines();
        tokio::spawn(async move {
            while let Ok(Some(l)) = lines.next_line().await {
                eprintln!("[game!] {l}");
            }
        });
    }

    let status = child.wait().await.context("等待游戏进程")?;
    println!("游戏退出,状态: {status}");
    Ok(())
}

/// Build the Microsoft auth client, mirroring the desktop's resolution order so
/// CLI login uses the same Azure app id: runtime `MC_MSA_CLIENT_ID` → compile-time
/// baked id → vanilla legacy id. The application (client) id is a public
/// identifier (device-code / public-client flow uses no secret).
pub(crate) fn msa_client() -> MsaClient {
    let runtime = std::env::var("MC_MSA_CLIENT_ID").ok();
    let baked = option_env!("MC_MSA_CLIENT_ID").map(str::to_string);
    match runtime.or(baked).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(id) => MsaClient::with_client_id(id),
        None => MsaClient::new(),
    }
}

/// 若当前选中的是外置(Yggdrasil)账号,启动前用 validate/refresh 续期并写回 store。
///
/// 先 `validate`:仍有效则直接复用现有 token(no-op);若失效(403)则用旧
/// access_token + 持久化的 client_token `refresh` 出新 token,原地更新并保持选中。
/// 非外置账号、缺少 client_token / base 时直接返回 `Ok(())`。
pub(crate) async fn refresh_selected_yggdrasil(store: &mut AccountStore) -> Result<()> {
    let (uuid, base, access_token, client_token) = match store.selected_account() {
        Some(a) if a.kind == AccountKind::Yggdrasil => {
            match (a.yggdrasil_base.clone(), a.client_token.clone()) {
                (Some(base), Some(ct)) => {
                    (a.uuid.clone(), base, a.access_token.clone(), ct)
                }
                // 缺 base 或 client_token(老数据)无法续期,交由游戏内校验兜底。
                _ => return Ok(()),
            }
        }
        _ => return Ok(()),
    };

    let client = YggdrasilClient::new(base.clone());
    // 仍有效就不动 token,避免无谓地让旧 token 失效。
    if client.validate(&access_token, &client_token).await? {
        return Ok(());
    }

    let refreshed = client.refresh(&access_token, &client_token).await?;
    let prev = store
        .selected_account()
        .cloned()
        .context("外置账号在续期过程中丢失")?;
    store.add_and_select(StoredAccount {
        kind: AccountKind::Yggdrasil,
        username: if refreshed.username.is_empty() { prev.username } else { refreshed.username },
        uuid: uuid.clone(),
        access_token: refreshed.access_token,
        refresh_token: None,
        xuid: String::new(),
        user_type: "msa".to_string(),
        owns_game: true,
        expires_at: None,
        client_token: Some(refreshed.client_token),
        yggdrasil_base: Some(base),
    })?;
    Ok(())
}

pub(crate) async fn cmd_login() -> Result<()> {
    let client = msa_client();
    let code = client.device_code_start().await.context("获取设备码")?;
    println!("\n请在浏览器打开:  {}", code.verification_uri);
    println!("输入代码:        {}\n", code.user_code);
    println!("等待授权…(完成登录后自动继续)");

    let token = client.poll_token(&code.device_code, code.interval).await?;
    let session = client.authenticate(&token.access_token).await?;

    let mut store = AccountStore::load(accounts_path())?;
    store.add_and_select(StoredAccount::from_microsoft(&session, token.refresh_token.clone()))?;
    println!("\n✓ 登录成功:{} ({})", session.username, session.uuid);
    Ok(())
}

/// 离线登录:由用户名派生稳定 UUID,落库为离线账号并选中。
pub(crate) fn cmd_login_offline(name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("用户名不能为空");
    }
    let session = offline_session(name);
    let mut store = AccountStore::load(accounts_path())?;
    store.add_and_select(StoredAccount::from_offline(&session))?;
    println!("✓ 已添加离线账号:{} ({})", session.username, session.uuid);
    Ok(())
}

/// 外置(Yggdrasil)登录:用 base + 用户名 + 密码登录皮肤站,落库为外置账号并选中,
/// 持久化 client_token(续期所需)与 base(启动时注入 authlib-injector 所需)。
pub(crate) async fn cmd_login_yggdrasil(base: &str, username: &str, password: &str) -> Result<()> {
    let base = base.trim();
    if base.is_empty() || username.trim().is_empty() {
        anyhow::bail!("皮肤站地址和用户名不能为空");
    }
    let client = YggdrasilClient::new(base);
    println!("在 {} 外置登录 {} …", client.base(), username.trim());
    let session = client.authenticate(username.trim(), password).await?;
    let mut store = AccountStore::load(accounts_path())?;
    store.add_and_select(StoredAccount::from_yggdrasil(&session, client.base().to_string()))?;
    println!("\n✓ 外置登录成功:{} ({})", session.username, session.uuid);
    Ok(())
}

pub(crate) fn cmd_accounts() -> Result<()> {
    let store = AccountStore::load(accounts_path())?;
    let accounts = store.list();
    if accounts.is_empty() {
        println!("没有已保存的账号。运行 `mc login` 添加微软账号。");
        return Ok(());
    }
    for a in accounts {
        println!(
            "{} {:<16} {:?} {}",
            if a.selected { "*" } else { " " },
            a.username,
            a.kind,
            a.uuid
        );
    }
    Ok(())
}
