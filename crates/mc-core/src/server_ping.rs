//! Minecraft **Server List Ping** (SLP) over the native protocol.
//!
//! Queries a server's status (online/players/latency/MOTD) before joining, the
//! same handshake the vanilla multiplayer list uses. Pure-TCP, no third-party
//! crate: we hand-roll the VarInt (7-bit LEB128) codec and the four packets of
//! the status sequence — Handshake, Status Request, Status Response, Ping/Pong.
//!
//! v1 scope: `host` or `host:port` only (default port 25565). SRV-record
//! resolution (`_minecraft._tcp`) is intentionally out of scope.
//!
//! Reference: <https://minecraft.wiki/w/Java_Edition_protocol/Server_List_Ping>.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Protocol version sent in the handshake. `-1` means "I'm only querying status"
/// and is accepted by every modern server regardless of its real version.
const PROTOCOL_VERSION: i32 = -1;
const DEFAULT_PORT: u16 = 25565;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_secs(3);
/// Guard against a malicious/garbled length prefix making us allocate the world.
const MAX_PACKET_LEN: usize = 2 * 1024 * 1024;

/// A server's reachability + status snapshot. Always returned (never an error):
/// on any failure `online` is `false` and the optional fields are `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ServerStatus {
    pub online: bool,
    pub players_online: Option<u32>,
    pub players_max: Option<u32>,
    pub motd: Option<String>,
    pub version: Option<String>,
    pub latency_ms: Option<u32>,
}

impl ServerStatus {
    fn offline() -> Self {
        ServerStatus::default()
    }
}

/// Ping a Minecraft server and return its status. Never errors: any
/// timeout / connection-refused / parse failure yields an offline status.
pub async fn ping_server(addr: &str) -> ServerStatus {
    match ping_inner(addr).await {
        Ok(status) => status,
        Err(_) => ServerStatus::offline(),
    }
}

async fn ping_inner(addr: &str) -> std::io::Result<ServerStatus> {
    let (host, port) = parse_addr(addr);

    let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "connect timeout"))??;
    let mut stream = stream;
    stream.set_nodelay(true).ok();

    // --- Handshake (0x00): protocol, host, port, next-state = 1 (status) ---
    let mut handshake = Vec::new();
    write_varint(&mut handshake, 0x00);
    write_varint(&mut handshake, PROTOCOL_VERSION);
    write_string(&mut handshake, &host);
    handshake.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut handshake, 1);
    write_packet(&mut stream, &handshake).await?;

    // --- Status Request (0x00, empty body) ---
    let mut request = Vec::new();
    write_varint(&mut request, 0x00);
    write_packet(&mut stream, &request).await?;

    // --- Status Response (0x00, VarInt JSON length, JSON) ---
    let payload = read_packet(&mut stream).await?;
    let mut cur = &payload[..];
    let packet_id = read_varint(&mut cur)?;
    if packet_id != 0x00 {
        return Err(invalid("unexpected status response id"));
    }
    let json_len = read_varint(&mut cur)? as usize;
    if json_len > cur.len() {
        return Err(invalid("status json length overruns packet"));
    }
    let json = std::str::from_utf8(&cur[..json_len]).map_err(|_| invalid("status json not utf-8"))?;
    let mut status = parse_status_json(json);

    // --- Ping/Pong (0x01 + i64 payload) for latency ---
    if let Ok(latency) = measure_latency(&mut stream).await {
        status.latency_ms = Some(latency);
    }

    status.online = true;
    Ok(status)
}

/// Send a Ping (0x01 + i64), read the Pong, return round-trip in ms.
async fn measure_latency(stream: &mut TcpStream) -> std::io::Result<u32> {
    let token: i64 = 0x0123_4567_89AB_CDEF;
    let mut ping = Vec::new();
    write_varint(&mut ping, 0x01);
    ping.extend_from_slice(&token.to_be_bytes());
    let start = Instant::now();
    write_packet(stream, &ping).await?;
    let pong = read_packet(stream).await?;
    let elapsed = start.elapsed().as_millis();
    // Best-effort: a well-behaved server echoes the same id+token; we don't fail
    // on a mismatch, latency is the only thing we actually need here.
    let _ = pong;
    Ok(elapsed.min(u32::MAX as u128) as u32)
}

/// Split `host` / `host:port`, defaulting to 25565. Bracketed IPv6 (`[::1]:25565`)
/// is handled so the colons inside the literal aren't mistaken for the port sep.
fn parse_addr(addr: &str) -> (String, u16) {
    let addr = addr.trim();
    if let Some(rest) = addr.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let host = &rest[..end];
            let after = &rest[end + 1..];
            let port = after.strip_prefix(':').and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
            return (host.to_string(), port);
        }
    }
    match addr.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(port) => (h.to_string(), port),
            Err(_) => (addr.to_string(), DEFAULT_PORT),
        },
        None => (addr.to_string(), DEFAULT_PORT),
    }
}

/// Parse the status JSON, flattening `description` (string OR chat-component
/// object) to plain text. Missing/odd fields degrade to `None` rather than fail.
fn parse_status_json(json: &str) -> ServerStatus {
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return ServerStatus::offline(),
    };
    ServerStatus {
        online: false, // set by caller once we know the socket worked
        players_online: value.get("players").and_then(|p| p.get("online")).and_then(|v| v.as_u64()).map(|n| n as u32),
        players_max: value.get("players").and_then(|p| p.get("max")).and_then(|v| v.as_u64()).map(|n| n as u32),
        version: value
            .get("version")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty()),
        motd: value.get("description").map(flatten_description).filter(|s| !s.is_empty()),
        latency_ms: None,
    }
}

/// Flatten a `description` (chat component) to plain text. Handles a bare string,
/// `{ "text": ".." }`, and the recursive `extra: [...]` component tree. Color /
/// formatting codes are dropped.
fn flatten_description(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => {
            let mut out = String::new();
            if let Some(serde_json::Value::String(t)) = map.get("text") {
                out.push_str(t);
            }
            if let Some(serde_json::Value::Array(extra)) = map.get("extra") {
                for child in extra {
                    out.push_str(&flatten_description(child));
                }
            }
            out
        }
        serde_json::Value::Array(items) => items.iter().map(flatten_description).collect(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Framing helpers
// ---------------------------------------------------------------------------

/// Prefix `body` with its VarInt length and write it.
async fn write_packet(stream: &mut TcpStream, body: &[u8]) -> std::io::Result<()> {
    let mut framed = Vec::with_capacity(body.len() + 5);
    write_varint(&mut framed, body.len() as i32);
    framed.extend_from_slice(body);
    timeout(IO_TIMEOUT, stream.write_all(&framed))
        .await
        .map_err(|_| invalid("write timeout"))??;
    Ok(())
}

/// Read one length-prefixed packet body (after the leading VarInt length).
async fn read_packet(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let len = read_varint_async(stream).await? as usize;
    if len == 0 || len > MAX_PACKET_LEN {
        return Err(invalid("packet length out of range"));
    }
    let mut buf = vec![0u8; len];
    timeout(IO_TIMEOUT, stream.read_exact(&mut buf))
        .await
        .map_err(|_| invalid("read timeout"))??;
    Ok(buf)
}

/// Read a VarInt straight off the socket, one byte at a time.
async fn read_varint_async(stream: &mut TcpStream) -> std::io::Result<i32> {
    let mut result: i32 = 0;
    for shift in 0..5 {
        let mut byte = [0u8; 1];
        timeout(IO_TIMEOUT, stream.read_exact(&mut byte))
            .await
            .map_err(|_| invalid("varint read timeout"))??;
        result |= ((byte[0] & 0x7F) as i32) << (7 * shift);
        if byte[0] & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err(invalid("varint too long"))
}

fn invalid(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string())
}

// ---------------------------------------------------------------------------
// VarInt / String codec (MC 7-bit LEB128)
// ---------------------------------------------------------------------------

/// Append `value` to `out` as a Minecraft VarInt (unsigned 7-bit groups, little
/// continuation bit). Negative ints use the full 5-byte two's-complement form.
fn write_varint(out: &mut Vec<u8>, value: i32) {
    let mut v = value as u32;
    loop {
        if v & !0x7F == 0 {
            out.push(v as u8);
            return;
        }
        out.push(((v & 0x7F) | 0x80) as u8);
        v >>= 7;
    }
}

/// Decode a VarInt from the front of `buf`, advancing it past the bytes read.
fn read_varint(buf: &mut &[u8]) -> std::io::Result<i32> {
    let mut result: i32 = 0;
    for shift in 0..5 {
        let byte = *buf.first().ok_or_else(|| invalid("varint truncated"))?;
        *buf = &buf[1..];
        result |= ((byte & 0x7F) as i32) << (7 * shift);
        if byte & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err(invalid("varint too long"))
}

/// Write a length-prefixed (VarInt) UTF-8 string.
fn write_string(out: &mut Vec<u8>, s: &str) {
    write_varint(out, s.len() as i32);
    out.extend_from_slice(s.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: i32) -> i32 {
        let mut buf = Vec::new();
        write_varint(&mut buf, v);
        let mut slice = &buf[..];
        read_varint(&mut slice).unwrap()
    }

    #[test]
    fn varint_roundtrips() {
        for v in [0, 1, 2, 127, 128, 255, 300, 25565, 2_147_483_647, -1, -2_147_483_648] {
            assert_eq!(roundtrip(v), v, "roundtrip failed for {v}");
        }
    }

    #[test]
    fn varint_known_encodings() {
        let mut buf = Vec::new();
        write_varint(&mut buf, 0);
        assert_eq!(buf, vec![0x00]);

        buf.clear();
        write_varint(&mut buf, 128);
        assert_eq!(buf, vec![0x80, 0x01]);

        buf.clear();
        write_varint(&mut buf, 255);
        assert_eq!(buf, vec![0xFF, 0x01]);

        buf.clear();
        write_varint(&mut buf, -1);
        assert_eq!(buf, vec![0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    }

    #[test]
    fn varint_truncated_errors() {
        let mut slice: &[u8] = &[0x80]; // continuation bit set but no next byte
        assert!(read_varint(&mut slice).is_err());
    }

    #[test]
    fn flattens_string_description() {
        let v = serde_json::json!("Hello MOTD");
        assert_eq!(flatten_description(&v), "Hello MOTD");
    }

    #[test]
    fn flattens_component_description() {
        let v = serde_json::json!({
            "text": "A ",
            "extra": [
                { "text": "Minecraft ", "color": "gold" },
                { "text": "Server", "bold": true }
            ]
        });
        assert_eq!(flatten_description(&v), "A Minecraft Server");
    }

    #[test]
    fn parses_full_status_json() {
        let json = r#"{
            "version": { "name": "1.20.1", "protocol": 763 },
            "players": { "max": 20, "online": 3 },
            "description": { "text": "Welcome!" }
        }"#;
        let s = parse_status_json(json);
        assert_eq!(s.version.as_deref(), Some("1.20.1"));
        assert_eq!(s.players_online, Some(3));
        assert_eq!(s.players_max, Some(20));
        assert_eq!(s.motd.as_deref(), Some("Welcome!"));
    }

    #[test]
    fn parse_addr_defaults_and_ports() {
        assert_eq!(parse_addr("mc.example.com"), ("mc.example.com".to_string(), 25565));
        assert_eq!(parse_addr("mc.example.com:25577"), ("mc.example.com".to_string(), 25577));
        assert_eq!(parse_addr("[::1]:25565"), ("::1".to_string(), 25565));
        assert_eq!(parse_addr("[::1]"), ("::1".to_string(), 25565));
    }
}
