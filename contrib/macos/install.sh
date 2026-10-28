#!/usr/bin/env bash
#
# ipv6-anchor 的 macOS 安装脚本：编译 → ad-hoc 签名 → 装二进制 → 写配置 → 注册自启。
#
#   ./contrib/macos/install.sh --hostname myhost.dynv6.net --token xxxxxxxx
#   ./contrib/macos/install.sh                       # 配置已存在时，只重装二进制和自启
#   ./contrib/macos/install.sh --system --hostname … # 开机即启（root LaunchDaemon，要 sudo）
#
# 卸载：./contrib/macos/uninstall.sh
#
# 所有路径都可以用 --prefix 改；默认全部落在用户目录里，除了 --system 之外不需要 sudo。

set -euo pipefail

LABEL="com.rusthing.ipv6-anchor"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

MODE="agent"
PREFIX=""
HOSTNAME_ARG=""
TOKEN_ARG=""
INTERVAL_ARG=""
START=1
SKIP_BUILD=0
# 参数会被下面的 while 循环 shift 光，报错提示里要复用原始参数
ORIG_ARGS=("$@")

usage() {
    cat <<'EOF'
用法：install.sh [选项]

  --hostname HOST   写入配置的 dynv6 主机名，例如 myhost.dynv6.net
  --token TOKEN     写入配置的 HTTP token（文件权限会被设成 600）
  --interval SECS   检查间隔，默认 300，最小 60
  --prefix DIR      二进制安装目录（默认 ~/.local/bin，--system 时 /usr/local/bin）
  --system          装成 LaunchDaemon：开机即启，root 运行，需要 sudo
  --no-start        只装文件，不注册 / 不启动
  --skip-build      跳过 cargo build，直接用 target/release/ipv6-anchor
  -h, --help        看这个
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --hostname) HOSTNAME_ARG="${2:?--hostname 后面要跟主机名}"; shift 2 ;;
        --token)    TOKEN_ARG="${2:?--token 后面要跟 token}"; shift 2 ;;
        --interval) INTERVAL_ARG="${2:?--interval 后面要跟秒数}"; shift 2 ;;
        --prefix)   PREFIX="${2:?--prefix 后面要跟目录}"; shift 2 ;;
        --system)   MODE="daemon"; shift ;;
        --no-start) START=0; shift ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        -h|--help)  usage; exit 0 ;;
        *) echo "未知参数：$1" >&2; usage >&2; exit 2 ;;
    esac
done

if [[ "$MODE" == "daemon" ]]; then
    if [[ $EUID -ne 0 ]]; then
        echo "装成 LaunchDaemon 需要 root，请用：sudo $0 ${ORIG_ARGS[*]}" >&2
        exit 1
    fi
    PREFIX="${PREFIX:-/usr/local/bin}"
    CONFIG="/Library/Application Support/ipv6-anchor/config.toml"
    LOG="/var/log/ipv6-anchor.log"
    PLIST_DIR="/Library/LaunchDaemons"
    DOMAIN="system"
    TEMPLATE="$HERE/com.rusthing.ipv6-anchor.daemon.plist.in"
    WORKDIR="/Library/Application Support/ipv6-anchor"
else
    PREFIX="${PREFIX:-$HOME/.local/bin}"
    CONFIG="$HOME/.config/ipv6-anchor/config.toml"
    LOG="$HOME/Library/Logs/ipv6-anchor.log"
    PLIST_DIR="$HOME/Library/LaunchAgents"
    DOMAIN="gui/$(id -u)"
    TEMPLATE="$HERE/com.rusthing.ipv6-anchor.plist.in"
    WORKDIR="$HOME"
fi

BIN="$PREFIX/ipv6-anchor"
PLIST="$PLIST_DIR/$LABEL.plist"

# ---------------------------------------------------------------- 1. 编译

if [[ $SKIP_BUILD -eq 0 ]]; then
    if ! command -v cargo >/dev/null 2>&1; then
        cat >&2 <<'EOF'
找不到 cargo。
  - 普通用户装：确认 ~/.cargo/bin 在 PATH 里
  - sudo 装（--system）：sudo 不继承 PATH，先手动 `cargo build --release`，
    再 `sudo ./install.sh --system --skip-build ...`
EOF
        exit 1
    fi
    echo "==> 编译（release）"
    cargo build --release --manifest-path "$REPO/Cargo.toml"
fi

BIN_SRC="$REPO/target/release/ipv6-anchor"
[[ -x "$BIN_SRC" ]] || { echo "没有 ${BIN_SRC}，去掉 --skip-build 重新编译" >&2; exit 1; }

# ---------------------------------------------------------------- 2. 签名

# profile 里开了 strip = true，链接器给 arm64 打的 ad-hoc 签名会被 strip 弄坏，
# 表现是运行时报 "code signature invalid" / 直接被内核杀掉。
# 本地自己编译、没走下载，所以 ad-hoc（--sign -）就够，不需要开发者证书。
echo "==> ad-hoc 签名"
codesign --force --sign - "$BIN_SRC"
codesign --verify --strict "$BIN_SRC"
echo "    签名有效"

# ---------------------------------------------------------------- 3. 装二进制

echo "==> 安装二进制到 $BIN"
install -d "$PREFIX"
install -m 755 "$BIN_SRC" "$BIN"

# ---------------------------------------------------------------- 4. 配置

if [[ -n "$HOSTNAME_ARG" || -n "$TOKEN_ARG" ]]; then
    echo "==> 写配置 $CONFIG"
    install -d "$(dirname "$CONFIG")"
    {
        echo "# ipv6-anchor 配置（由 install.sh 生成）"
        echo "hostname = \"${HOSTNAME_ARG:-myhost.dynv6.net}\""
        [[ -n "$TOKEN_ARG" ]] && echo "token = \"$TOKEN_ARG\""
        [[ -n "$INTERVAL_ARG" ]] && echo "interval = $INTERVAL_ARG"
    } > "$CONFIG"
    chmod 600 "$CONFIG"
elif [[ ! -f "$CONFIG" ]]; then
    echo "==> 生成配置模板 $CONFIG"
    install -d "$(dirname "$CONFIG")"
    cp "$REPO/config.example.toml" "$CONFIG"
    chmod 600 "$CONFIG"
fi

# 还是模板里的示例域名，说明没填过，装了也是白跑
NEED_EDIT=0
if grep -q 'myhost\.dynv6\.net' "$CONFIG" 2>/dev/null; then
    NEED_EDIT=1
fi

# ---------------------------------------------------------------- 5. 注册自启

echo "==> 注册 ${LABEL}（${DOMAIN}）"
install -d "$PLIST_DIR"
install -d "$(dirname "$LOG")"
# WorkingDirectory 不存在的话 launchd 会直接拒绝加载
install -d "$WORKDIR"
sed -e "s|@LABEL@|$LABEL|g" \
    -e "s|@BIN@|$BIN|g" \
    -e "s|@CONFIG@|$CONFIG|g" \
    -e "s|@WORKDIR@|$WORKDIR|g" \
    -e "s|@LOG@|$LOG|g" \
    "$TEMPLATE" > "$PLIST"
chmod 644 "$PLIST"
plutil -lint "$PLIST" >/dev/null
echo "    plist 语法 OK：$PLIST"

if [[ $START -eq 0 || $NEED_EDIT -eq 1 ]]; then
    echo
    echo "文件已就位，但还没启动。"
    [[ $NEED_EDIT -eq 1 ]] && echo "先把 $CONFIG 里的 hostname / token 填好（示例域名不能直接用）。"
    echo "然后执行："
    echo "    launchctl bootstrap $DOMAIN \"$PLIST\""
    echo "    launchctl kickstart -k $DOMAIN/$LABEL"
    exit 0
fi

# 重复安装时先摘掉旧的，否则 bootstrap 会报 "already bootstrapped"
launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true

if ! launchctl bootstrap "$DOMAIN" "$PLIST"; then
    cat >&2 <<EOF

launchctl 拒绝了这次加载。最常见的原因不是 plist 写错（上面刚 lint 过），
而是当前 shell 不在 Aqua 会话里——ssh、IDE 集成终端、被别的 agent 拉起的
shell 都会返回 "5: Input/output error"。换 Terminal.app 重跑这一条就行：

    launchctl bootstrap $DOMAIN "$PLIST"

文件都已经就位，不需要重装。
EOF
    exit 1
fi

launchctl enable "$DOMAIN/$LABEL"

echo
echo "==> 已启动。接下来："
echo "    状态    launchctl print $DOMAIN/$LABEL"
echo "    日志    tail -f \"$LOG\""
echo "    重启    launchctl kickstart -k $DOMAIN/$LABEL"
echo "    停掉    launchctl bootout $DOMAIN/$LABEL"
