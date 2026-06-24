#!/usr/bin/env bash
# gallery.sh — 起真实 Tauri 窗口,逐页截图,生成 HTML 画廊(仅 macOS)。
#
# 原理:用 MC_GALLERY=1 启动 debug 二进制;前端挂载后(src/gallery/runner.ts)逐页
# 切换 → 后端 gallery_capture 用 `screencapture` 抓 main 原生窗口 → gallery_build 写
# <data_dir>/gallery/index.html 并自动打开。截的是真实渲染 + 真实数据,不走 web/mock。
#
# 用法:
#   scripts/gallery.sh          # 重新构建前端 + 二进制,再起画廊
#   scripts/gallery.sh ui       # 跳过 cargo 构建(仅前端变更时,二进制需已存在)
#
# 首次运行可能需要在「系统设置 → 隐私与安全性 → 屏幕录制」里给终端/二进制授权,
# 否则 screencapture 抓到的是黑屏。授权后重跑即可。
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESKTOP="$ROOT/desktop"
TAURI="$DESKTOP/src-tauri"
BIN="$TAURI/target/debug/mc-launcher-desktop"
APP_LOG="/tmp/mc-gallery.log"

SKIP_BUILD=0
for a in "$@"; do
  case "$a" in
    ui|nobuild|fast) SKIP_BUILD=1 ;;
    help|-h|--help) sed -n '2,17p' "$0"; exit 0 ;;
    *) echo "⚠ unknown arg: $a (use: ui | help)" ;;
  esac
done

[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
if [ -f "$TAURI/.env" ]; then
  set -a; source "$TAURI/.env"; set +a
fi

# 1) 前端 bundle(直接跑二进制加载的是 frontendDist,不是 vite dev) ----------------
echo "→ npm build (frontend bundle)…"
if ! ( cd "$DESKTOP" && npm run build ); then
  echo "✗ frontend build failed"; exit 1
fi

# 2) debug 二进制 ----------------------------------------------------------------
if [ "$SKIP_BUILD" = 1 ]; then
  echo "↷ skipping cargo build (ui mode)"
else
  echo "→ cargo build (desktop shell)…"
  if ! ( cd "$TAURI" && cargo build ); then
    echo "✗ build failed"; exit 1
  fi
fi
if [ ! -x "$BIN" ]; then
  echo "✗ binary not found: $BIN (run without 'ui' first)"; exit 1
fi

# 3) 停掉旧实例,起画廊模式 --------------------------------------------------------
OLD=$(pgrep -f mc-launcher-desktop)
if [ -n "$OLD" ]; then
  echo "→ stopping old app (pid $(echo "$OLD" | tr '\n' ' '))"
  kill $OLD 2>/dev/null; sleep 1
fi

echo "→ launching in gallery mode (MC_GALLERY=1)…"
MC_GALLERY=1 nohup "$BIN" > "$APP_LOG" 2>&1 &
PID=$!
echo "  pid $PID · log: $APP_LOG"

# 4) 等截图流程跑完(逐页等待累加约 10s,留足余量) ---------------------------------
echo "→ capturing pages… (约 15s,期间请勿遮挡窗口)"
for _ in $(seq 1 30); do
  sleep 1
  if grep -q "画廊已生成" "$APP_LOG" 2>/dev/null; then
    GAL=$(grep "画廊已生成" "$APP_LOG" | tail -1 | sed 's/.*画廊已生成: //')
    echo ""
    echo "✓ 画廊已生成并打开: $GAL"
    exit 0
  fi
done

echo ""
echo "⚠ 未在日志里看到完成标记。检查 $APP_LOG;若 screencapture 报权限错误,"
echo "  到「系统设置 → 隐私与安全性 → 屏幕录制」授权后重跑。"
grep -iE "gallery|screencapture|画廊|error" "$APP_LOG" 2>/dev/null | tail -10
