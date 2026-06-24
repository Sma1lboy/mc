//! The launch pipeline: resolve the profile, ensure files & Java, extract
//! natives, build the command line, and spawn the game. Also handles installing
//! a fresh version. Implements `docs/01-launch-chain.md`.

pub mod command;

pub use command::{build_launch_command, join_classpath, LaunchVars};

use std::io::Read;
use std::path::{Path, PathBuf};

use mc_types::{AuthSession, ManifestVersion, Progress};
use tokio::sync::watch;

use crate::download::{checksum, DownloadItem, Downloader};
use crate::error::{CoreError, IoResultExt, Result};
use crate::instance::Instance;
use crate::java::{self, JavaInstall};
use crate::meta;
use crate::paths::{ensure_dir, GamePaths};
use crate::version::{resolve_profile, LaunchProfile, RuntimeContext, DEFAULT_LIBRARIES_MAVEN};

/// Inputs to a launch.
pub struct LaunchSpec {
    pub instance: Instance,
    pub session: AuthSession,
    /// Explicit Java path; when `None` the launcher auto-detects.
    pub java_path: Option<PathBuf>,
    pub launcher_name: String,
    pub launcher_version: String,
    /// When false, skip the network "ensure files" step (offline play).
    pub online: bool,
    /// Where to auto-provision a JRE when no compatible local Java is found
    /// (`<data_dir>/java`, holding `jre-{major}/` per major). `None` disables
    /// auto-install and a missing Java surfaces as [`CoreError::JavaNotFound`].
    pub runtimes_dir: Option<PathBuf>,
    /// Global default Java path (from settings). Used when neither the explicit
    /// override nor the per-instance config specify one — i.e. precedence is
    /// `java_path` > instance config > this > auto-detect > auto-provision.
    pub global_java_path: Option<PathBuf>,
    /// Extra JVM arguments injected just before the main class (e.g. the
    /// authlib-injector `-javaagent` for Yggdrasil accounts). Empty for most launches.
    pub extra_jvm_args: Vec<String>,
    /// One-shot server to auto-join for this launch only (`host` or `host:port`),
    /// overriding the instance config's `server`. `None` keeps the configured value.
    /// Used by the saved-servers "quick join" so joining doesn't permanently rewrite
    /// the instance config.
    pub server_override: Option<String>,
}

/// A loader closure that reads `versions/<id>/<id>.json` from disk for the
/// inheritance chain.
fn disk_loader(paths: &GamePaths) -> impl FnMut(&str) -> Result<String> + '_ {
    move |id: &str| {
        let path = paths.version_json(id);
        std::fs::read_to_string(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => CoreError::VersionNotFound(id.to_string()),
            _ => CoreError::io(path, e),
        })
    }
}

/// Resolve a fully-merged profile from the version jsons already on disk.
pub fn resolve_disk_profile(paths: &GamePaths, id: &str) -> Result<LaunchProfile> {
    resolve_profile(id, disk_loader(paths))
}

/// Absolute classpath jar list: applicable (non-native) library jars plus the
/// version jar. Native bundles are extracted, never placed on the classpath.
pub fn build_classpath(profile: &LaunchProfile, paths: &GamePaths, ctx: &RuntimeContext) -> Vec<String> {
    let mut jars: Vec<String> = Vec::new();
    for lib in crate::version::classpath_libraries(&profile.libraries, ctx) {
        if let Some(file) = lib.classpath_file(DEFAULT_LIBRARIES_MAVEN) {
            let abs = paths.libraries_dir().join(&file.path);
            jars.push(abs.to_string_lossy().into_owned());
        }
    }
    jars.push(paths.version_jar(&profile.id).to_string_lossy().into_owned());
    jars
}

/// Extract every applicable native jar into the per-version natives directory.
pub fn extract_natives(profile: &LaunchProfile, paths: &GamePaths, ctx: &RuntimeContext) -> Result<()> {
    let natives_dir = paths.natives_dir(&profile.id);
    ensure_dir(&natives_dir)?;

    // Old-style (pre-1.19): native carried by a regular library via its `natives` map.
    for lib in &profile.libraries {
        if !lib.applies(ctx) {
            continue;
        }
        if let Some(native) = lib.native_file(ctx) {
            let jar_path = paths.libraries_dir().join(&native.path);
            if jar_path.exists() {
                extract_native_jar(&jar_path, &natives_dir, &native.extract_exclude)?;
            }
        }
    }

    // New-style (1.19+): dedicated native library entries, one per arch.
    for lib in crate::version::select_native_libraries(&profile.libraries, ctx) {
        let Some(file) = lib.classpath_file(DEFAULT_LIBRARIES_MAVEN) else { continue };
        let jar_path = paths.libraries_dir().join(&file.path);
        if jar_path.exists() {
            let exclude = lib.extract.as_ref().map(|e| e.exclude.clone()).unwrap_or_default();
            extract_native_jar(&jar_path, &natives_dir, &exclude)?;
        }
    }
    Ok(())
}

fn extract_native_jar(jar: &Path, dest: &Path, exclude: &[String]) -> Result<()> {
    let file = std::fs::File::open(jar).with_path(jar)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Zip(e.to_string()))?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| CoreError::Zip(e.to_string()))?;
        let name = entry.name().to_string();
        if entry.is_dir() || name.starts_with("META-INF/") {
            continue;
        }
        if exclude.iter().any(|ex| name.starts_with(ex.trim_end_matches('/'))) {
            continue;
        }
        // Only extract actual native libraries.
        let is_native = name.ends_with(".dll")
            || name.ends_with(".so")
            || name.ends_with(".dylib")
            || name.ends_with(".jnilib");
        if !is_native {
            continue;
        }
        let out_path = dest.join(Path::new(&name).file_name().unwrap_or_default());
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf).map_err(|e| CoreError::Zip(e.to_string()))?;
        std::fs::write(&out_path, buf).with_path(&out_path)?;
    }
    Ok(())
}

/// Build the asset-index/object download items and write the index file to disk.
async fn ensure_assets(
    dl: &Downloader,
    paths: &GamePaths,
    profile: &LaunchProfile,
) -> Result<Vec<DownloadItem>> {
    let Some(idx) = &profile.asset_index else { return Ok(vec![]) };

    let index = meta::fetch_asset_index(dl, idx).await?;

    // Persist the index json the game reads from `assets/indexes/<id>.json`.
    let index_path = paths.asset_indexes_dir().join(format!("{}.json", idx.id));
    ensure_dir(paths.asset_indexes_dir().as_path())?;
    let raw = serde_json::to_vec(&index).map_err(|e| CoreError::Parse { what: "asset index".into(), source: e })?;
    crate::fs::write_atomic(&index_path, &raw)?;

    Ok(meta::asset_download_items(&index, paths))
}

/// Verify all required files and download whatever is missing or corrupt.
/// This is both the "prepare to launch" and the "repair" path.
pub async fn ensure_files(
    dl: &Downloader,
    paths: &GamePaths,
    profile: &LaunchProfile,
    ctx: &RuntimeContext,
    progress: Option<watch::Sender<Progress>>,
) -> Result<()> {
    let mut items = meta::library_download_items(profile, paths, ctx);
    if let Some(jar) = meta::client_jar_item(profile, paths) {
        items.push(jar);
    }
    items.extend(ensure_assets(dl, paths, profile).await?);

    // Only download what's actually broken (rayon-parallel checksum scan).
    let broken_idx = checksum::find_broken(&items);
    let to_fetch: Vec<DownloadItem> = broken_idx.into_iter().map(|i| items[i].clone()).collect();

    if to_fetch.is_empty() {
        return Ok(());
    }
    dl.download_all(to_fetch, progress).await
}

/// Install a vanilla version: fetch its json, persist it, then ensure all files.
pub async fn install_version(
    dl: &Downloader,
    paths: &GamePaths,
    entry: &ManifestVersion,
    progress: Option<watch::Sender<Progress>>,
) -> Result<()> {
    let raw = meta::fetch_version_json(dl, entry).await?;
    let dir = paths.version_dir(&entry.id);
    ensure_dir(&dir)?;
    let json_path = paths.version_json(&entry.id);
    crate::fs::write_atomic(&json_path, raw.as_bytes())?;

    // Vanilla/Forge installers expect a launcher_profiles.json at the root.
    let _ = crate::fs::ensure_launcher_profiles(paths.root());

    let profile = resolve_disk_profile(paths, &entry.id)?;
    let ctx = RuntimeContext::for_launch();
    ensure_files(dl, paths, &profile, &ctx, progress).await
}

/// Pick a Java install: explicit override → config → auto-detect by required major
/// → auto-provision (download a Temurin JRE of that major) when nothing local fits.
///
/// Auto-provisioning is what makes "just press Play" hold across the Java-8/17/21
/// split: a user with only Java 21 installed can still launch a 1.12 instance. It's
/// idempotent (a previously downloaded `jre-{major}` is reused, no re-download) and
/// only kicks in when [`LaunchSpec::runtimes_dir`] is set.
async fn resolve_java(
    spec: &LaunchSpec,
    profile: &LaunchProfile,
    mc_version: &str,
    dl: &Downloader,
    progress: Option<&watch::Sender<Progress>>,
) -> Result<PathBuf> {
    if let Some(p) = &spec.java_path {
        return Ok(p.clone());
    }
    let config = spec.instance.load_config()?;
    if let Some(p) = &config.java_path {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    // 实例没单独指定 Java 时,跟随全局设置的 Java 路径(让设置页「Java 路径」真正生效)。
    if let Some(p) = &spec.global_java_path {
        if !p.as_os_str().is_empty() {
            return Ok(p.clone());
        }
    }
    let major = java::required_major(mc_version, profile.java_major);
    let installs: Vec<JavaInstall> = java::detect_all().await;
    if let Some(j) = java::select(&installs, major) {
        return Ok(j.path.clone());
    }

    // Nothing local matches the required major — provision one if we're allowed to.
    match &spec.runtimes_dir {
        Some(dir) => {
            if let Some(tx) = progress {
                let _ = tx.send(Progress::new(format!("下载 Java {major} 运行时(首次较慢)")));
            }
            java::install::install_jre(dl, dir, major).await
        }
        None => Err(CoreError::JavaNotFound { major }),
    }
}

/// Run the full launch pipeline and return the spawned child process.
pub async fn launch(
    spec: LaunchSpec,
    dl: &Downloader,
    progress: Option<watch::Sender<Progress>>,
) -> Result<tokio::process::Child> {
    let paths = spec.instance.paths();
    let ctx = RuntimeContext::for_launch();
    let version_id = spec.instance.version_id().to_string();

    // 0. guard against paths that silently break Java (the infamous '!').
    let issues = crate::fs::check_problematic_path(paths.root());
    if crate::fs::has_blocking_path_issue(&issues) {
        let msg = issues.iter().map(|i| i.message.clone()).collect::<Vec<_>>().join(" ");
        return Err(CoreError::Launch(msg));
    }

    // 1. resolve merged profile
    let profile = resolve_disk_profile(&paths, &version_id)?;
    let mc_version = profile.assets_id.clone().unwrap_or_else(|| version_id.clone());
    let mut config = spec.instance.load_config()?;
    // 一次性服务器覆盖(快速进入某存档服务器):只影响本次启动,不改写实例配置。
    if let Some(server) = spec.server_override.as_ref().filter(|s| !s.trim().is_empty()) {
        config.server = Some(server.clone());
    }

    // 2. ensure files (skipped offline)
    if spec.online {
        ensure_files(dl, &paths, &profile, &ctx, progress.clone()).await?;
    }

    // 3. Java (auto-provision a matching JRE if none is installed)
    let java_path = resolve_java(&spec, &profile, &mc_version, dl, progress.as_ref()).await?;

    // 4. natives
    extract_natives(&profile, &paths, &ctx)?;

    // 5. command line
    let game_dir = spec.instance.dir();
    ensure_dir(&game_dir)?;
    let classpath = join_classpath(&build_classpath(&profile, &paths, &ctx));
    let vars = LaunchVars {
        game_dir: command::path_str(&game_dir),
        assets_root: command::path_str(&paths.assets_dir()),
        assets_index: profile.assets_id.clone().unwrap_or_else(|| "legacy".into()),
        natives_dir: command::path_str(&paths.natives_dir(&profile.id)),
        libraries_dir: command::path_str(&paths.libraries_dir()),
        classpath,
        launcher_name: spec.launcher_name.clone(),
        launcher_version: spec.launcher_version.clone(),
    };
    let mut args = build_launch_command(&profile, &config, &spec.session, &vars, &ctx);
    // 注入额外 JVM 参数(如外置登录的 authlib-injector -javaagent):必须在主类**之前**,
    // 否则 JVM 会把它当成程序参数。插到主类位置前一格。
    if !spec.extra_jvm_args.is_empty() {
        let main_pos = args.iter().position(|a| a == &profile.main_class).unwrap_or(0);
        for (i, extra) in spec.extra_jvm_args.iter().enumerate() {
            args.insert(main_pos + i, extra.clone());
        }
    }

    // 6. spawn
    if let Some(tx) = &progress {
        let _ = tx.send(Progress::new("启动游戏进程"));
    }
    let child = tokio::process::Command::new(&java_path)
        .args(&args)
        .current_dir(&game_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| CoreError::Launch(format!("无法启动 Java 进程 {}: {e}", java_path.display())))?;

    Ok(child)
}
