# easytier-relay — kobeMC 联机大厅自建中继节点

「我们提供 host 点」那种联机方式的兜底中继 / 会合节点(EasyTier external-node)。

## 它是什么 / 不是什么
- **是**:一个独立的网络 daemon(easytier-core),监听 TCP，给打洞失败的成员**中继**游戏流量、并帮所有人**会合**(交换公网地址触发打洞)。
- **不是**:mc-server 的一个 HTTP 端点。它是另一个进程 / 另一个 Railway service。

## 两种联机方式怎么落到 external-node
- **P2P**:客户端用 EasyTier 公共节点会合 → 多数人直连，零成本（mc-server 默认返回 `tcp://public.easytier.cn:11010`，可用 `MC_LOBBY_P2P_NODE` 覆盖）。
- **我们的 host 点**:客户端用**本节点**会合 + 中继。把本节点的公网地址(Railway TCP Proxy 给的 `host:port`)填进 mc-server 的 `MC_LOBBY_RELAY` 环境变量(形如 `tcp://relay.example.com:12345`)。两种都返回，客户端按模式切。

## 已部署（Railway · kobemc-server 项目）
- **Service**：`easytier-relay`（与 mc-server / Postgres 同一个 Railway 项目）。
- **公网地址（TCP Proxy）**：`maglev.proxy.rlwy.net:42516` → 容器端口 `11010`（已 ACTIVE，地址稳定）。
- **mc-server 已配**：`MC_LOBBY_RELAY=tcp://maglev.proxy.rlwy.net:42516`（暂存，随 mc-server 下次部署 / restart 生效）。

## ⚠️ 一次性手动步骤(Railway CLI 做不到,得在 Dashboard 点)
这个 service 和 mc-server 共用仓库根的 `railway.json`(那份是 mc-server 的:`dockerfilePath: Dockerfile`
+ `healthcheckPath: /v1/health`)。`railway up` 会照那份 build 出 **mc-server** 镜像并卡在
`/v1/health` 健康检查——不是我们要的 relay。CLI 无法按 service 覆盖 Dockerfile 路径 / 去掉健康检查
(`RAILWAY_DOCKERFILE_PATH` 变量被根 `railway.json` 的显式值盖掉)。

**Dashboard 里把 `easytier-relay` service 的 Root Directory 设为 `infra/easytier-relay`**,它就会读
本目录的 `railway.json`(Dockerfile builder、无健康检查)、按本目录的 Dockerfile build 出真正的 relay。
设好后 **Redeploy** 这个 service 即可。

## 部署 / 重新部署流程
1. (一次性)Dashboard 设 Root Directory = `infra/easytier-relay`(见上)。
2. 本机在本目录跑 `railway up -s easytier-relay`,或在 Dashboard 点 Redeploy。
3. relay 上线后,**restart mc-server** 让 `MC_LOBBY_RELAY` 生效:`railway redeploy -s mc-server`。
4. TCP Proxy 已开(端口 11010 → `maglev.proxy.rlwy.net:42516`);换地址时同步更新 mc-server 的
   `MC_LOBBY_RELAY=tcp://<host>:<port>`。

> 注:`nc -z <host> <port>` 只探到 Railway proxy 边缘(边缘先 accept 再回源),**不能**证明 relay
> 真在跑;真验证要么看 service 状态 = Online,要么用 easytier 客户端握手。

仅 P2P 也能完整工作（不配 `MC_LOBBY_RELAY` 时 lobby 接口只返回公共节点）。中继是给穿透失败的少数兜底。

`--relay-network-whitelist kobe-*` 限定只为我们的领域网络(`network_name = kobe-<realm_id>`)中继，防白嫖。
