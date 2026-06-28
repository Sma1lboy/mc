# easytier-relay — kobeMC 联机大厅自建中继节点

「我们提供 host 点」那种联机方式的兜底中继 / 会合节点(EasyTier external-node)。

## 它是什么 / 不是什么
- **是**:一个独立的网络 daemon(easytier-core),监听 TCP，给打洞失败的成员**中继**游戏流量、并帮所有人**会合**(交换公网地址触发打洞)。
- **不是**:mc-server 的一个 HTTP 端点。它是另一个进程 / 另一个 Railway service。

## 两种联机方式怎么落到 external-node
- **P2P**:客户端用 EasyTier 公共节点会合 → 多数人直连，零成本（mc-server 默认返回 `tcp://public.easytier.cn:11010`，可用 `MC_LOBBY_P2P_NODE` 覆盖）。
- **我们的 host 点**:客户端用**本节点**会合 + 中继。把本节点的公网地址(Railway TCP Proxy 给的 `host:port`)填进 mc-server 的 `MC_LOBBY_RELAY` 环境变量(形如 `tcp://relay.example.com:12345`)。两种都返回，客户端按模式切。

## 部署到 Railway
1. 新建 service，Source 指向本目录的 Dockerfile（或 `railway up` 本目录）。
2. 给它开 **TCP Proxy**，目标端口 **11010**。
3. 拿到 Railway 给的公网 `host:port`，写进 mc-server 的 `MC_LOBBY_RELAY=tcp://<host>:<port>`。

仅 P2P 也能完整工作（不配 `MC_LOBBY_RELAY` 时 lobby 接口只返回公共节点）。中继是给穿透失败的少数兜底。

`--relay-network-whitelist kobe-*` 限定只为我们的领域网络(`network_name = kobe-<realm_id>`)中继，防白嫖。
