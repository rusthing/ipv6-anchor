#!/usr/bin/env bash
#
# 卸载 ipv6-anchor 的 macOS 自启与二进制。
#
#   ./contrib/macos/uninstall.sh            # 摘掉自启 + 删二进制，配置和日志留着
#   ./contrib/macos/uninstall.sh --purge    # 连配置和日志一起删
#   sudo ./contrib/macos/uninstall.sh --system --purge
#
# 跟 install.sh 用同样的默认路径，路径改过就用 --prefix / 手动删。

set -euo pipefail

LABEL="com.rusthing.ipv6-anchor"
MODE="agent"
PREFIX=""
PURGE=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --system) MODE="daemon"; shift ;;
        --prefix) PREFIX="${2:?}"; shift 2 ;;
        --purge)  PURGE=1; shift ;;
        -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
        *) echo "未知参数：$1" >&2; exit 2 ;;
    esac
done

if [[ "$MODE" == "daemon" ]]; then
    [[ $EUID -eq 0 ]] || { echo "请用 sudo 执行" >&2; exit 1; }
    PREFIX="${PREFIX:-/usr/local/bin}"
    CONFIG="/Library/Application Support/ipv6-anchor/config.toml"
    LOG="/var/log/ipv6-anchor.log"
    PLIST="/Library/LaunchDaemons/$LABEL.plist"
    DOMAIN="system"
else
    PREFIX="${PREFIX:-$HOME/.local/bin}"
    CONFIG="$HOME/.config/ipv6-anchor/config.toml"
    LOG="$HOME/Library/Logs/ipv6-anchor.log"
    PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
    DOMAIN="gui/$(id -u)"
fi

echo "==> 停止并摘掉自启"
launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || echo "    （本来就没在跑）"
rm -f "$PLIST"

echo "==> 删除二进制 $PREFIX/ipv6-anchor"
rm -f "$PREFIX/ipv6-anchor"

if [[ $PURGE -eq 1 ]]; then
    echo "==> 删除配置与日志"
    rm -f "$CONFIG" "$LOG"
    rmdir "$(dirname "$CONFIG")" 2>/dev/null || true
fi

echo "完成。"
