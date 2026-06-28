//! 检测 Minecraft「对局域网开放」(Open to LAN)。
//!
//! 当玩家把单机世界对局域网开放,MC 会在随机端口起一个内置服务器,并**每 ~1.5s** 往
//! 组播组 **224.0.2.60:4445** 广播一个发现包,载荷形如:
//!
//! ```text
//! [MOTD]<世界标题>[/MOTD][AD]<端口>[/AD]
//! ```
//!
//! host 自己这台机器也会收到自己的广播(MC 客户端本就监听 4445 来在「多人游戏」里列出
//! 局域网世界),所以我们能在**同一台机**上探到 host 刚开的端口——这正是联机大厅 host
//! 流程需要的:探到端口后,把 `<虚拟IP>:<端口>` 发布给领域成员一键加入。
//!
//! 本模块两部分:[`parse_lan_announce`] 纯解析(可单测),[`detect_lan_port`] 实际加入
//! 组播监听一小段时间取端口。两者都**容错**:解析不出 / 绑定失败 / 超时 → `None`,绝不 panic。

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};

/// MC 局域网发现的组播组与端口(固定值,见 Minecraft 源码 `LanServerPinger`)。
const LAN_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 2, 60);
const LAN_PORT: u16 = 4445;

/// 纯解析一个 MC 局域网发现包 `[MOTD]<motd>[/MOTD][AD]<port>[/AD]`。
///
/// 容错:缺 `[AD]…[/AD]` 段、端口非数字 / 越界、整体不是这个格式 → `None`。`[MOTD]` 段
/// 缺失时 motd 取空串(端口才是必须的)。允许包前后有多余文本(只要能定位到两对标记)。
pub fn parse_lan_announce(payload: &str) -> Option<(String, u16)> {
    fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
        let start = s.find(open)? + open.len();
        let rest = &s[start..];
        let end = rest.find(close)?;
        Some(&rest[..end])
    }

    // 端口是必须项;解析不出合法 u16 → 整体失败。
    let port_str = between(payload, "[AD]", "[/AD]")?.trim();
    let port: u16 = port_str.parse().ok()?;
    if port == 0 {
        return None;
    }
    let motd = between(payload, "[MOTD]", "[/MOTD]").unwrap_or("").trim().to_string();
    Some((motd, port))
}

/// 加入组播 224.0.2.60:4445 监听,直到读到一个能解析出端口的包或 `timeout` 超时。
///
/// 用 `socket2` 建 socket 以便在 bind 前打开 `SO_REUSEADDR`(+ unix 的 `SO_REUSEPORT`):
/// host 机上 MC 客户端本身已经绑了 4445 来发现局域网世界,不复用地址就会 `AddrInUse`。
/// 任何一步失败(建 socket / bind / join / 读)都**容错**返回 `None`,绝不 panic。
pub async fn detect_lan_port(timeout: Duration) -> Option<u16> {
    let socket = bind_multicast_listener().ok()?;
    let socket = tokio::net::UdpSocket::from_std(socket).ok()?;

    let mut buf = [0u8; 2048];
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let recv = tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await;
        match recv {
            // 超时:本轮没等到包。
            Err(_) => return None,
            // socket 错误:不 panic,直接放弃本次探测。
            Ok(Err(_)) => return None,
            Ok(Ok((n, _from))) => {
                let payload = String::from_utf8_lossy(&buf[..n]);
                if let Some((_motd, port)) = parse_lan_announce(&payload) {
                    return Some(port);
                }
                // 不是我们认识的包,继续等下一拍(MC 每 ~1.5s 再发)。
            }
        }
    }
}

/// 建一个绑定 `0.0.0.0:4445`、加入 MC 局域网组播组、非阻塞的 std UDP socket。
fn bind_multicast_listener() -> std::io::Result<std::net::UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    let bind_addr: SocketAddr = SocketAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, LAN_PORT));
    socket.bind(&bind_addr.into())?;
    socket.join_multicast_v4(&LAN_GROUP, &Ipv4Addr::UNSPECIFIED)?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_packet() {
        let (motd, port) = parse_lan_announce("[MOTD]Jackson's World[/MOTD][AD]52137[/AD]").unwrap();
        assert_eq!(motd, "Jackson's World");
        assert_eq!(port, 52137);
    }

    #[test]
    fn parses_with_surrounding_noise() {
        // MC 实测包有时前后带空白 / 控制字符;只要两对标记在就该解出。
        let (motd, port) = parse_lan_announce("\u{0}[MOTD]Hi[/MOTD][AD]25565[/AD]\n").unwrap();
        assert_eq!(motd, "Hi");
        assert_eq!(port, 25565);
    }

    #[test]
    fn missing_ad_segment_is_none() {
        assert!(parse_lan_announce("[MOTD]No port here[/MOTD]").is_none());
        assert!(parse_lan_announce("[AD]12345").is_none()); // 缺闭合标记
    }

    #[test]
    fn bad_port_is_none() {
        assert!(parse_lan_announce("[MOTD]x[/MOTD][AD]notanumber[/AD]").is_none());
        assert!(parse_lan_announce("[MOTD]x[/MOTD][AD]70000[/AD]").is_none()); // u16 越界
        assert!(parse_lan_announce("[MOTD]x[/MOTD][AD]0[/AD]").is_none()); // 0 端口无意义
    }

    #[test]
    fn garbage_is_none() {
        assert!(parse_lan_announce("").is_none());
        assert!(parse_lan_announce("totally unrelated text").is_none());
    }

    #[test]
    fn motd_optional_port_required() {
        // 没有 MOTD 段但端口齐全 → motd 空串,端口照取。
        let (motd, port) = parse_lan_announce("[AD]40000[/AD]").unwrap();
        assert_eq!(motd, "");
        assert_eq!(port, 40000);
    }
}
