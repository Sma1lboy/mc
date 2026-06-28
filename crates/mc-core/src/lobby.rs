//! 联机大厅(EasyTier room)凭据。一个领域 = 一个虚拟局域网房间:所有成员从 mc-server
//! 拿到**相同**的 `network_name` + `network_secret`(成员鉴权后才发),进同一个 EasyTier
//! 网络;`nodes` 是会合 / 中继的 external-node 列表(P2P 公共节点 + 可选的我们自建中继)。
//!
//! 这里只负责**取凭据**(P1)。真正拉起 EasyTier(建 TUN、加入网络)走后续阶段的特权 helper。

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::server::ServerClient;

/// 一个 EasyTier external/relay 节点。`kind`:`"p2p"` = 公共共享节点(打洞后直连,我们零成本);
/// `"hosted"` = 我们自建的中继(host 点,打洞失败时兜底)。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LobbyNode {
    pub kind: String,
    pub name: String,
    pub addr: String,
}

/// 某领域的联机大厅凭据(EasyTier 房间)。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LobbyCreds {
    pub network_name: String,
    pub network_secret: String,
    #[serde(default)]
    pub nodes: Vec<LobbyNode>,
}

impl ServerClient {
    /// 取某领域的联机大厅凭据(成员鉴权;非成员 403)。
    pub async fn realm_lobby(&self, realm_id: &str) -> Result<LobbyCreds> {
        self.get_json(&format!("/v1/realms/{realm_id}/lobby")).await
    }
}

// ============================================================================
// 纯逻辑(P2):构建 easytier-core 启动参数、挑选会合节点、解析 easytier-cli peer 表。
// 这些都不触碰进程/TUN/特权,可被单测完整覆盖;真正的 spawn/elevation 留给桌面层(thin)。
// ============================================================================

/// 把任意字符串净化成合法的 EasyTier hostname:仅保留 `[A-Za-z0-9_-]`,其余替换为 `-`。
/// 空结果回退到 `"peer"`(EasyTier 拒绝空 hostname)。
fn sanitize_hostname(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "peer".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 构建 `easytier-core` 的命令行参数:加入 `network_name` + `network_secret` 标识的虚拟网络,
/// 以 `node_addr` 作为会合 / 中继 external-node,`--dhcp` 自动分配 `10.x` 虚拟 IP,`--hostname`
/// 让本机在别人那张 peer 表里有个可读名字。hostname 会被净化为 `[A-Za-z0-9_-]`。
pub fn easytier_core_args(creds: &LobbyCreds, node_addr: &str, hostname: &str) -> Vec<String> {
    vec![
        "--network-name".into(),
        creds.network_name.clone(),
        "--network-secret".into(),
        creds.network_secret.clone(),
        "--external-node".into(),
        node_addr.to_string(),
        "--dhcp".into(),
        "--hostname".into(),
        sanitize_hostname(hostname),
    ]
}

/// 按 `mode`(`"p2p"` / `"hosted"`)挑会合节点:优先返回 `kind == mode` 的节点,缺失时回退到
/// 第一个节点(凭据保证至少有一个 p2p 公共节点);完全没有节点时返回 `None`。
pub fn pick_node<'a>(creds: &'a LobbyCreds, mode: &str) -> Option<&'a LobbyNode> {
    creds
        .nodes
        .iter()
        .find(|n| n.kind == mode)
        .or_else(|| creds.nodes.first())
}

/// `easytier-cli peer` 表里的一行(一个对端,或本机自己那行)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LobbyPeer {
    pub ipv4: Option<String>,
    pub hostname: String,
    pub cost: String,
    pub lat_ms: Option<u32>,
    pub nat_type: Option<String>,
}

/// 联机大厅当前状态:是否在跑、本机虚拟 IP、对端列表(不含本机自己那行)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LobbyStatus {
    pub running: bool,
    pub virtual_ip: Option<String>,
    pub peers: Vec<LobbyPeer>,
}

/// 把单元格里的「空值占位」(EasyTier 常用 `-` / `N/A` / 空串)归一成 `None`。
fn cell_opt(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() || t == "-" || t.eq_ignore_ascii_case("n/a") || t == "/" {
        None
    } else {
        Some(t.to_string())
    }
}

/// 容错解析 `easytier-cli peer` 的表格输出。**按表头列名定位列**(不假设固定列序),因此即便
/// EasyTier 改了列顺序也能存活;识别并跳过 box-drawing / ascii 边框与分隔行。无法解析 → `vec![]`。
///
/// 典型表头:`ipv4 | hostname | cost | lat_ms | loss_rate | rx_bytes | tx_bytes | tunnel_proto |
/// nat_type | id | version`。本机自己那行 `cost = Local`、`ipv4` 即本机虚拟 IP。
pub fn parse_peer_table(cli_output: &str) -> Vec<LobbyPeer> {
    // 把一行表格按竖线(ascii `|` 或 box-drawing 竖线)切成单元格,去掉首尾因边框产生的空段。
    fn split_cells(line: &str) -> Vec<String> {
        let cells: Vec<String> = line
            .split(['|', '│', '┃'])
            .map(|c| c.trim().to_string())
            .collect();
        // 去掉因首尾竖线造成的前后空单元格,但保留中间的(中间空单元格是真实的空值)。
        let mut start = 0;
        let mut end = cells.len();
        if cells.first().map(|c| c.is_empty()).unwrap_or(false) {
            start = 1;
        }
        if end > start && cells.last().map(|c| c.is_empty()).unwrap_or(false) {
            end -= 1;
        }
        cells[start..end].to_vec()
    }

    // 一行是否纯边框 / 分隔线(只含 `-=+|` 与各种 box-drawing 字符、空白)。
    fn is_border(line: &str) -> bool {
        let t = line.trim();
        !t.is_empty()
            && t.chars().all(|c| {
                c.is_whitespace()
                    || matches!(c, '-' | '=' | '+' | '|')
                    || ('\u{2500}'..='\u{257F}').contains(&c) // box drawing block
            })
    }

    // 1) 找表头行:含 "hostname" 且含 "cost"(列名定位的锚)。
    let header_line = cli_output.lines().find(|l| {
        let low = l.to_lowercase();
        low.contains("hostname") && low.contains("cost")
    });
    let Some(header_line) = header_line else {
        return vec![];
    };

    let headers: Vec<String> = split_cells(header_line)
        .into_iter()
        .map(|h| h.to_lowercase())
        .collect();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let ipv4_i = col("ipv4");
    let host_i = col("hostname");
    let cost_i = col("cost");
    // 延迟列名在不同版本里可能是 lat_ms / latency_ms / latency。
    let lat_i = col("lat_ms").or_else(|| col("latency_ms")).or_else(|| col("latency"));
    let nat_i = col("nat_type").or_else(|| col("nat"));

    // hostname 与 cost 是必需锚;缺失说明这不是我们认识的表 → 放弃。
    let (Some(host_i), Some(cost_i)) = (host_i, cost_i) else {
        return vec![];
    };

    let mut out = Vec::new();
    let mut seen_header = false;
    for line in cli_output.lines() {
        if std::ptr::eq(line, header_line) {
            seen_header = true;
            continue;
        }
        if !seen_header {
            continue;
        }
        if line.trim().is_empty() || is_border(line) {
            continue;
        }
        let cells = split_cells(line);
        // 数据行至少要覆盖到 cost 列。
        if cells.len() <= cost_i {
            continue;
        }
        let get = |i: Option<usize>| -> Option<&str> { i.and_then(|i| cells.get(i)).map(|s| s.as_str()) };
        let hostname = cells.get(host_i).cloned().unwrap_or_default();
        let cost = cells.get(cost_i).cloned().unwrap_or_default();
        // 整行都空(纯分隔残留)跳过。
        if hostname.is_empty() && cost.is_empty() {
            continue;
        }
        let lat_ms = get(lat_i).and_then(cell_opt).and_then(|s| s.parse::<u32>().ok());
        out.push(LobbyPeer {
            ipv4: get(ipv4_i).and_then(cell_opt),
            hostname,
            cost,
            lat_ms,
            nat_type: get(nat_i).and_then(cell_opt),
        });
    }
    out
}

/// 由整张 peer 表派生 [`LobbyStatus`]:`virtual_ip` 取 `cost == "Local"` 那行的 ipv4,
/// `peers` 是其余(非本机)行,`running = true`。
pub fn status_from_peers(peers: Vec<LobbyPeer>) -> LobbyStatus {
    let virtual_ip = peers
        .iter()
        .find(|p| p.cost.eq_ignore_ascii_case("local"))
        .and_then(|p| p.ipv4.clone());
    let others: Vec<LobbyPeer> = peers
        .into_iter()
        .filter(|p| !p.cost.eq_ignore_ascii_case("local"))
        .collect();
    LobbyStatus {
        running: true,
        virtual_ip,
        peers: others,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> LobbyCreds {
        LobbyCreds {
            network_name: "kobe-r1".into(),
            network_secret: "s3cr3t".into(),
            nodes: vec![
                LobbyNode { kind: "p2p".into(), name: "public".into(), addr: "tcp://public.easytier.cn:11010".into() },
                LobbyNode { kind: "hosted".into(), name: "ours".into(), addr: "tcp://relay.kobe.gg:11010".into() },
            ],
        }
    }

    #[test]
    fn core_args_shape() {
        let c = creds();
        let args = easytier_core_args(&c, "tcp://public.easytier.cn:11010", "Jackson's Mac!");
        assert_eq!(
            args,
            vec![
                "--network-name", "kobe-r1",
                "--network-secret", "s3cr3t",
                "--external-node", "tcp://public.easytier.cn:11010",
                "--dhcp",
                "--hostname", "Jackson-s-Mac", // 非法字符 → '-',首尾 '-' 去掉
            ]
        );
    }

    #[test]
    fn hostname_empty_falls_back() {
        let c = creds();
        let args = easytier_core_args(&c, "addr", "！！！");
        let i = args.iter().position(|a| a == "--hostname").unwrap();
        assert_eq!(args[i + 1], "peer");
    }

    #[test]
    fn pick_node_modes_and_fallback() {
        let c = creds();
        assert_eq!(pick_node(&c, "p2p").unwrap().name, "public");
        assert_eq!(pick_node(&c, "hosted").unwrap().name, "ours");
        // 请求一个不存在的 kind → 回退到第一个节点。
        assert_eq!(pick_node(&c, "nope").unwrap().name, "public");
        // 无节点 → None。
        let empty = LobbyCreds { network_name: "n".into(), network_secret: "s".into(), nodes: vec![] };
        assert!(pick_node(&empty, "p2p").is_none());
    }

    #[test]
    fn parse_realistic_table() {
        // 含 box-drawing 边框 + 表头 + 本机(Local)行 + 一个 p2p 直连对端 + 一个 relay 对端。
        let sample = "\
┌─────────────┬──────────┬───────┬────────┬───────────┬──────────┬──────────┬──────────────┬──────────┬──────┬─────────┐
│ ipv4        │ hostname │ cost  │ lat_ms │ loss_rate │ rx_bytes │ tx_bytes │ tunnel_proto │ nat_type │ id   │ version │
├─────────────┼──────────┼───────┼────────┼───────────┼──────────┼──────────┼──────────────┼──────────┼──────┼─────────┤
│ 10.144.0.1  │ my-mac   │ Local │ -      │ -         │ 0        │ 0        │ -            │ FullCone │ 1001 │ 2.0.3   │
│ 10.144.0.2  │ alice-pc │ p2p   │ 12     │ 0.00      │ 1024     │ 2048     │ udp          │ FullCone │ 1002 │ 2.0.3   │
│ 10.144.0.3  │ bob-box  │ relay │ 88     │ 0.01      │ 512      │ 256      │ tcp          │ Symmetric│ 1003 │ 2.0.3   │
└─────────────┴──────────┴───────┴────────┴───────────┴──────────┴──────────┴──────────────┴──────────┴──────┴─────────┘";
        let peers = parse_peer_table(sample);
        assert_eq!(peers.len(), 3);

        let local = &peers[0];
        assert_eq!(local.cost, "Local");
        assert_eq!(local.ipv4.as_deref(), Some("10.144.0.1"));
        assert_eq!(local.lat_ms, None);

        let alice = &peers[1];
        assert_eq!(alice.hostname, "alice-pc");
        assert_eq!(alice.cost, "p2p");
        assert_eq!(alice.lat_ms, Some(12));
        assert_eq!(alice.nat_type.as_deref(), Some("FullCone"));

        let bob = &peers[2];
        assert_eq!(bob.cost, "relay");
        assert_eq!(bob.lat_ms, Some(88));

        // status 派生:本机虚拟 IP 来自 Local 行;peers 去掉 Local。
        let status = status_from_peers(peers);
        assert!(status.running);
        assert_eq!(status.virtual_ip.as_deref(), Some("10.144.0.1"));
        assert_eq!(status.peers.len(), 2);
        assert!(status.peers.iter().all(|p| p.cost != "Local"));
    }

    #[test]
    fn parse_survives_column_reorder() {
        // 列序与文档不同(cost 在前),仍按列名定位。
        let sample = "\
cost  | hostname | ipv4       | lat_ms | nat_type
Local | me       | 10.0.0.1   | -      | -
p2p   | peer1    | 10.0.0.2   | 5      | FullCone";
        let peers = parse_peer_table(sample);
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].cost, "Local");
        assert_eq!(peers[0].ipv4.as_deref(), Some("10.0.0.1"));
        assert_eq!(peers[1].cost, "p2p");
        assert_eq!(peers[1].ipv4.as_deref(), Some("10.0.0.2"));
        assert_eq!(peers[1].lat_ms, Some(5));
    }

    #[test]
    fn parse_empty_or_garbage() {
        assert!(parse_peer_table("").is_empty());
        assert!(parse_peer_table("not a table at all\njust text").is_empty());
        // 有表头但无数据行。
        assert!(parse_peer_table("ipv4 | hostname | cost | lat_ms").is_empty());
    }

    #[test]
    fn status_no_local_row() {
        let peers = vec![LobbyPeer {
            ipv4: Some("10.0.0.9".into()),
            hostname: "x".into(),
            cost: "p2p".into(),
            lat_ms: Some(3),
            nat_type: None,
        }];
        let s = status_from_peers(peers);
        assert!(s.running);
        assert_eq!(s.virtual_ip, None);
        assert_eq!(s.peers.len(), 1);
    }
}
