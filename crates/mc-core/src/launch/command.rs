//! Build the final `java` argument vector from a resolved [`LaunchProfile`].
//! Pure and testable: takes everything as inputs, returns the argument list.
//! See `docs/01-launch-chain.md` §5.

use std::collections::HashMap;
use std::path::Path;

use mc_types::AuthSession;

use crate::instance::InstanceConfig;
use crate::version::{rules_allow, Argument, LaunchProfile, RuntimeContext, StringOrList};

/// Everything needed to substitute the `${...}` placeholders in the argument
/// templates.
pub struct LaunchVars {
    pub game_dir: String,
    pub assets_root: String,
    pub assets_index: String,
    pub natives_dir: String,
    pub libraries_dir: String,
    pub classpath: String,
    pub launcher_name: String,
    pub launcher_version: String,
    /// Resolved Minecraft version (e.g. `1.20.1`), used to pick the right
    /// auto-join flag: `--quickPlayMultiplayer` (1.20+) vs legacy `--server`/`--port`.
    pub mc_version: String,
}

/// Build the complete argument list that follows the java executable:
/// `[jvm args...] mainClass [game args...]`.
pub fn build_launch_command(
    profile: &LaunchProfile,
    config: &InstanceConfig,
    session: &AuthSession,
    vars: &LaunchVars,
    ctx: &RuntimeContext,
) -> Vec<String> {
    let subst = placeholder_map(profile, session, vars);
    let mut out: Vec<String> = Vec::new();

    // ---- JVM arguments ----
    out.push(format!("-Xmx{}M", config.memory_mb.max(512)));
    out.push(format!("-Xms{}M", (config.memory_mb / 2).max(256)));

    if profile.jvm_args.is_empty() {
        // Pre-1.13 versions have no structured jvm args: supply the essentials.
        out.push(format!("-Djava.library.path={}", vars.natives_dir));
        out.push("-cp".to_string());
        out.push(vars.classpath.clone());
    } else {
        out.extend(eval_arguments(&profile.jvm_args, ctx, &subst));
    }

    // User-provided extra JVM args always apply.
    out.extend(config.jvm_args.iter().cloned());

    // ---- main class ----
    out.push(profile.main_class.clone());

    // ---- game arguments ----
    if !profile.game_args.is_empty() {
        out.extend(eval_arguments(&profile.game_args, ctx, &subst));
    } else if let Some(legacy) = &profile.legacy_arguments {
        for tok in legacy.split_whitespace() {
            out.push(substitute(tok, &subst));
        }
    }

    // ---- resolution / fullscreen / server ----
    if config.fullscreen {
        out.push("--fullscreen".to_string());
    } else {
        if let Some(w) = config.width {
            out.push("--width".to_string());
            out.push(w.to_string());
        }
        if let Some(h) = config.height {
            out.push("--height".to_string());
            out.push(h.to_string());
        }
    }
    if let Some(server) = &config.server {
        if let Some((host, port)) = parse_server(server) {
            if supports_quick_play(&vars.mc_version) {
                // 1.20+ 的一键进服参数,把地址直接交给游戏自动连接。
                out.push("--quickPlayMultiplayer".to_string());
                out.push(format!("{host}:{port}"));
            } else {
                // 1.20 之前没有 quickPlay,回退到 legacy 的 --server/--port。
                out.push("--server".to_string());
                out.push(host);
                out.push("--port".to_string());
                out.push(port.to_string());
            }
        }
    }

    out.extend(config.game_args.iter().cloned());
    out
}

fn placeholder_map(
    profile: &LaunchProfile,
    session: &AuthSession,
    vars: &LaunchVars,
) -> HashMap<&'static str, String> {
    let mut m = HashMap::new();
    m.insert("auth_player_name", session.username.clone());
    m.insert("auth_uuid", session.uuid.clone());
    m.insert("auth_access_token", session.access_token.clone());
    m.insert("auth_session", format!("token:{}:{}", session.access_token, session.uuid));
    m.insert("auth_xuid", session.xuid.clone());
    m.insert("user_type", session.user_type.clone());
    m.insert("user_properties", "{}".to_string());
    m.insert("clientid", String::new());
    m.insert("version_name", profile.id.clone());
    m.insert("version_type", "release".to_string());
    m.insert("game_directory", vars.game_dir.clone());
    m.insert("assets_root", vars.assets_root.clone());
    m.insert("game_assets", vars.assets_root.clone());
    m.insert("assets_index_name", vars.assets_index.clone());
    m.insert("natives_directory", vars.natives_dir.clone());
    m.insert("library_directory", vars.libraries_dir.clone());
    m.insert("classpath", vars.classpath.clone());
    m.insert("classpath_separator", mc_types::Os::current().classpath_separator().to_string());
    m.insert("launcher_name", vars.launcher_name.clone());
    m.insert("launcher_version", vars.launcher_version.clone());
    m
}

/// Evaluate a structured argument list: keep rule-passing entries, substitute
/// placeholders.
fn eval_arguments(
    args: &[Argument],
    ctx: &RuntimeContext,
    subst: &HashMap<&'static str, String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Argument::Plain(s) => out.push(substitute(s, subst)),
            Argument::Conditional { rules, value } => {
                if rules_allow(rules, ctx) {
                    let values = match value {
                        StringOrList::One(s) => vec![s.clone()],
                        StringOrList::Many(v) => v.clone(),
                    };
                    for v in values {
                        out.push(substitute(&v, subst));
                    }
                }
            }
        }
    }
    out
}

/// Replace every `${key}` for which we have a value; unknown placeholders are
/// left untouched.
fn substitute(input: &str, subst: &HashMap<&'static str, String>) -> String {
    if !input.contains("${") {
        return input.to_string();
    }
    let mut result = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find('}') {
            let key = &after[..end];
            match subst.get(key) {
                Some(val) => result.push_str(val),
                None => {
                    result.push_str("${");
                    result.push_str(key);
                    result.push('}');
                }
            }
            rest = &after[end + 1..];
        } else {
            result.push_str(&rest[start..]);
            rest = "";
        }
    }
    result.push_str(rest);
    result
}

/// Build the OS-appropriate classpath string from absolute jar paths.
pub fn join_classpath(jars: &[String]) -> String {
    let sep = mc_types::Os::current().classpath_separator();
    jars.join(&sep.to_string())
}

fn parse_server(s: &str) -> Option<(String, u16)> {
    // 带方括号的 IPv6 字面量：`[::1]` 或 `[::1]:25565`。保留方括号，
    // 这样调用方 `{host}:{port}` 重新拼接后仍是合法地址。
    if let Some(end) = s.strip_prefix('[').and_then(|_| s.find(']')) {
        let host = &s[..=end];
        return match &s[end + 1..] {
            "" => Some((host.to_string(), 25565)),
            rest => rest.strip_prefix(':')?.parse().ok().map(|port| (host.to_string(), port)),
        };
    }
    // 普通 host:port —— 只在最后一个冒号处拆分；无冒号则用默认端口。
    match s.rsplit_once(':') {
        Some((h, p)) => p.parse().ok().map(|port| (h.to_string(), port)),
        None => Some((s.to_string(), 25565)),
    }
}

/// Whether this Minecraft version understands `--quickPlayMultiplayer` (added in
/// 1.20 / 23w14a). Pre-1.20 releases must use the legacy `--server`/`--port` pair.
/// Versions we can't parse as `1.x` (snapshots, exotic ids) default to the modern
/// flag — the launcher overwhelmingly runs recent builds.
fn supports_quick_play(mc_version: &str) -> bool {
    let mut parts = mc_version.trim().split('.');
    if parts.next().map(str::trim) != Some("1") {
        return true;
    }
    match parts.next().and_then(|s| {
        let digits: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<u32>().ok()
    }) {
        Some(minor) => minor >= 20,
        None => true,
    }
}

/// Helper to map a path to a string for variable use.
pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::VersionJson;

    fn session() -> AuthSession {
        AuthSession {
            username: "Steve".into(),
            uuid: "1234".into(),
            access_token: "tok".into(),
            user_type: "msa".into(),
            xuid: "xuid1".into(),
        }
    }

    fn vars() -> LaunchVars {
        LaunchVars {
            game_dir: "/g".into(),
            assets_root: "/g/assets".into(),
            assets_index: "5".into(),
            natives_dir: "/g/natives".into(),
            libraries_dir: "/g/libraries".into(),
            classpath: "/g/a.jar:/g/client.jar".into(),
            launcher_name: "mc-launcher".into(),
            launcher_version: "0.1".into(),
            mc_version: "1.20.1".into(),
        }
    }

    #[test]
    fn substitutes_placeholders() {
        let m = {
            let mut m = HashMap::new();
            m.insert("auth_player_name", "Steve".to_string());
            m
        };
        assert_eq!(substitute("--user ${auth_player_name}!", &m), "--user Steve!");
        assert_eq!(substitute("${unknown}", &m), "${unknown}");
    }

    #[test]
    fn parse_server_handles_ipv6_and_hosts() {
        // 带方括号的 IPv6 + 端口
        assert_eq!(parse_server("[::1]:25565"), Some(("[::1]".to_string(), 25565)));
        // 带方括号的 IPv6，无端口 -> 默认端口
        assert_eq!(parse_server("[::1]"), Some(("[::1]".to_string(), 25565)));
        // 普通 host:port
        assert_eq!(parse_server("mc.example.com:25565"), Some(("mc.example.com".to_string(), 25565)));
        assert_eq!(parse_server("1.2.3.4:25577"), Some(("1.2.3.4".to_string(), 25577)));
        // 裸 host，无端口 -> 默认端口
        assert_eq!(parse_server("mc.example.com"), Some(("mc.example.com".to_string(), 25565)));
    }

    #[test]
    fn quick_play_gates_on_version() {
        assert!(supports_quick_play("1.20.1"));
        assert!(supports_quick_play("1.20"));
        assert!(supports_quick_play("1.21.4"));
        assert!(!supports_quick_play("1.19.4"));
        assert!(!supports_quick_play("1.8.9"));
        assert!(!supports_quick_play("1.16.5"));
        // 无法解析为 1.x 的(快照/异常 id)默认现代标志。
        assert!(supports_quick_play("23w14a"));
    }

    #[test]
    fn server_uses_quick_play_for_modern() {
        let vj = VersionJson::parse(
            r#"{"id":"1.20.1","mainClass":"M","minecraftArguments":"--username ${auth_player_name}","libraries":[]}"#,
        )
        .unwrap();
        let profile = LaunchProfile::from_chain(&[vj]);
        let cfg = InstanceConfig { server: Some("mc.example.com:25577".into()), ..Default::default() };
        let mut v = vars();
        v.mc_version = "1.20.1".into();
        let cmd = build_launch_command(&profile, &cfg, &session(), &v, &RuntimeContext::default());
        let joined = cmd.join(" ");
        assert!(joined.contains("--quickPlayMultiplayer mc.example.com:25577"));
        assert!(!joined.contains("--server"));
    }

    #[test]
    fn server_uses_legacy_for_old() {
        let vj = VersionJson::parse(
            r#"{"id":"1.8","mainClass":"M","minecraftArguments":"--username ${auth_player_name}","libraries":[]}"#,
        )
        .unwrap();
        let profile = LaunchProfile::from_chain(&[vj]);
        let cfg = InstanceConfig { server: Some("mc.example.com:25577".into()), ..Default::default() };
        let mut v = vars();
        v.mc_version = "1.8.9".into();
        let cmd = build_launch_command(&profile, &cfg, &session(), &v, &RuntimeContext::default());
        let joined = cmd.join(" ");
        assert!(joined.contains("--server mc.example.com --port 25577"));
        assert!(!joined.contains("--quickPlayMultiplayer"));
    }

    #[test]
    fn legacy_arguments_build() {
        let vj = VersionJson::parse(
            r#"{"id":"1.8","mainClass":"net.minecraft.client.main.Main","minecraftArguments":"--username ${auth_player_name} --uuid ${auth_uuid}","libraries":[]}"#,
        )
        .unwrap();
        let profile = LaunchProfile::from_chain(&[vj]);
        let cfg = InstanceConfig::default();
        let cmd = build_launch_command(&profile, &cfg, &session(), &vars(), &RuntimeContext::default());
        assert!(cmd.contains(&"net.minecraft.client.main.Main".to_string()));
        let joined = cmd.join(" ");
        assert!(joined.contains("--username Steve"));
        assert!(joined.contains("--uuid 1234"));
        assert!(joined.contains("-Djava.library.path=/g/natives"));
    }
}
