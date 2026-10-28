#!/usr/bin/env bash
#
# 把 ipv6-anchor 包成 macOS 的 .app（LSUIElement 后台程序，没有 Dock 图标）。
#
#   ./contrib/macos/make-app.sh                # 只做本机架构
#   ./contrib/macos/make-app.sh --universal    # Intel + Apple Silicon 合一的通用二进制
#
# 产物：dist/ipv6-anchor.app
#
# 什么时候需要 .app 而不是裸二进制？
#   - 想双击运行 / 拖进「系统设置 → 通用 → 登录项」；
#   - 想签 Developer ID 后公证分发给别人。
# 只给自己一台机器用的话，裸二进制 + LaunchAgent（install.sh）更省事，
# 而且能带命令行参数；.app 从 Finder 启动时拿不到参数，只能靠
# ~/.config/ipv6-anchor/config.toml 里的配置。

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
APP_NAME="ipv6-anchor"
BUNDLE_ID="com.rusthing.ipv6-anchor"
UNIVERSAL=0

[[ "${1:-}" == "--universal" ]] && UNIVERSAL=1

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/Cargo.toml" | head -1)"
OUT="$REPO/dist/$APP_NAME.app"

# ---------------------------------------------------------------- 1. 编译

if [[ $UNIVERSAL -eq 1 ]]; then
    echo "==> 编译 x86_64 + arm64"
    rustup target add x86_64-apple-darwin aarch64-apple-darwin >/dev/null
    cargo build --release --manifest-path "$REPO/Cargo.toml" --target x86_64-apple-darwin
    cargo build --release --manifest-path "$REPO/Cargo.toml" --target aarch64-apple-darwin
    echo "==> lipo 合并"
    rm -rf "$REPO/dist"
    mkdir -p "$REPO/dist"
    # 注意顺序：lipo 会作废签名，所以必须先合再签
    lipo -create \
        "$REPO/target/x86_64-apple-darwin/release/$APP_NAME" \
        "$REPO/target/aarch64-apple-darwin/release/$APP_NAME" \
        -output "$REPO/dist/$APP_NAME"
    BIN_SRC="$REPO/dist/$APP_NAME"
else
    echo "==> 编译（release，本机架构）"
    cargo build --release --manifest-path "$REPO/Cargo.toml"
    BIN_SRC="$REPO/target/release/$APP_NAME"
fi

# ---------------------------------------------------------------- 2. 目录结构

echo "==> 组装 $OUT"
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"
install -m 755 "$BIN_SRC" "$OUT/Contents/MacOS/$APP_NAME"
cp "$REPO/config.example.toml" "$OUT/Contents/Resources/ipv6-anchor.toml.example"

cat > "$OUT/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>ipv6-anchor</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key>
    <string>$APP_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <!-- 没有窗口也没有 Dock 图标，纯后台进程 -->
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

plutil -lint "$OUT/Contents/Info.plist" >/dev/null

# ---------------------------------------------------------------- 3. 签名

# 本地用：ad-hoc 即可。要分发就换成
#   codesign --force --options runtime --timestamp \
#            --sign "Developer ID Application: 你的名字 (TEAMID)" "$OUT"
echo "==> ad-hoc 签名 .app"
codesign --force --deep --sign - "$OUT"
codesign --verify --strict --verbose=2 "$OUT"

echo
echo "完成：$OUT  （${VERSION}，$(lipo -archs "$OUT/Contents/MacOS/$APP_NAME" 2>/dev/null || echo "单架构")）"
echo
echo "跑起来之前先把配置放好（.app 从 Finder 启动拿不到命令行参数）："
echo "    mkdir -p ~/.config/ipv6-anchor"
echo "    cp $REPO/config.example.toml ~/.config/ipv6-anchor/config.toml"
echo "    chmod 600 ~/.config/ipv6-anchor/config.toml   # 填好 hostname / token"
echo
echo "然后：open $OUT        双击也行"
echo "开机自启：系统设置 → 通用 → 登录项 → 添加 $OUT"
echo "          （别和 install.sh 装的 LaunchAgent 同时开，会跑两份）"
