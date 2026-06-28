# kobeMC 联机大厅的自建中继 / 会合节点(EasyTier external-node)。
# 部署成一个独立 Railway service(TCP Proxy 暴露 11010)。这是「我们提供 host 点」那一种
# 联机方式的兜底:打洞失败的成员把流量经它中继。--no-tun 因为它只转发不需要虚拟网卡;
# --relay-network-whitelist 限定只为我们的领域网络(kobe-*)中继,避免被白嫖。
FROM debian:bookworm-slim

ARG EASYTIER_VERSION=v2.6.4
ARG EASYTIER_ASSET=easytier-linux-x86_64-${EASYTIER_VERSION}.zip

RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates unzip \
 && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL "https://github.com/EasyTier/EasyTier/releases/download/${EASYTIER_VERSION}/${EASYTIER_ASSET}" -o /tmp/et.zip \
 && unzip /tmp/et.zip -d /opt \
 && rm /tmp/et.zip \
 && ln -s /opt/easytier-linux-x86_64/easytier-core /usr/local/bin/easytier-core

# EasyTier external-node 监听端口(在 Railway 用 TCP Proxy 映射到这个端口)。
EXPOSE 11010

CMD ["easytier-core", "--listeners", "tcp://0.0.0.0:11010", "--no-tun", "--relay-network-whitelist", "kobe-*"]
