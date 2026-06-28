#!/usr/bin/env bash
#
# dev-second.sh — 起「第二个」启动器实例,用于本地同时测两个 kobeMC 账号。
#
# 原理:kobe 会话只活在进程内存(reqwest cookie jar,不落盘),所以两个进程 = 两份
# 独立登录态,各登一个号即可。再用 portable 标记给第二个实例一个独立 data 目录,免得
# 两个进程抢同一份 settings/instances。二进制用改名副本,这样 dev-app.sh 的 pgrep
# 杀进程(只匹配 mc-launcher-desktop)不会误杀它,你重建账号 A 时账号 B 窗口照常活着。
#
#   scripts/dev-app.sh        # 账号 A(默认 data 目录)—— 先把它跑起来(顺带构建二进制)
#   scripts/dev-second.sh     # 账号 B(/tmp/kobeB,独立 data)
#   scripts/dev-second.sh /tmp/kobeC   # 想要第三个号就换个目录名
#
# 注意:登录态不跨重启,每个窗口启动后都要各自重新登录 / 注册一个号。
# 重建了账号 A 的二进制后,想让 B 也用新代码:再跑一次本脚本(会刷新副本)。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAURI="$ROOT/desktop/src-tauri"
BIN="$TAURI/target/debug/mc-launcher-desktop"
DIR="${1:-/tmp/kobeB}"

[ -x "$BIN" ] || { echo "✗ 二进制不存在,先跑 scripts/dev-app.sh 构建一次:$BIN"; exit 1; }

# 与账号 A 指向同一台 dev 服务器(MC_SERVER_URL),否则两个号不在同一后端、互相看不到。
[ -f "$TAURI/.env" ] && { set -a; source "$TAURI/.env"; set +a; }

mkdir -p "$DIR"
touch "$DIR/portable.txt"          # portable 标记 → data 落到 $DIR/launcher-data
cp -f "$BIN" "$DIR/kobe-second"    # 改名副本:dev-app.sh 的 pgrep 杀不到它

echo "→ 第二个实例"
echo "  data:   $DIR/launcher-data"
echo "  server: ${MC_SERVER_URL:-<server.rs 的 Railway 生产默认>}"
nohup "$DIR/kobe-second" > "$DIR/app.log" 2>&1 &
echo "  pid $!  ·  日志:$DIR/app.log"
