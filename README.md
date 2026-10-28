# ipv6-anchor

用 dynv6 更新本机的 IPv6 DDNS 记录。整个程序就是**一个 HTTP GET**。

```
GET https://ipv6.dynv6.com/api/update?hostname=myhost.dynv6.net&ipv6=auto&token=…
```

## 为什么这么简单

dynv6 的 `ipv6=auto` 会直接拿**这条连接的源 IPv6 地址**写进 AAAA，所以：

- 本机不用探测地址（不用枚举网卡、不用调回显服务）；
- 地址有没有变由 dynv6 自己判断，返回 `addresses updated` / `addresses unchanged`，本地不用存状态文件；
- 前缀同理，`ipv6prefix=auto` 让 dynv6 从源地址推 /64。

配套的两个细节，代码里都硬性保证：

1. **端点用 `ipv6.dynv6.com`**——这个域名只有 AAAA 记录（没有 A），连接必然走 IPv6；用 `dynv6.com` 可能落到 IPv4，`auto` 就拿不到 v6 地址了。
2. **不走环境代理**——`ureq` 3.5 的 `Config::default()` 会无条件调 `Proxy::try_from_env()`（`proxy-from-env` 特性开关已经没了，`default-features = false` 也关不掉），所以代码里显式 `.proxy(None)`。一旦请求经代理，dynv6 看到的是代理 IP，`auto` 就废了。

## 安装

```bash
cargo install ipv6-anchor      # 从 crates.io
```

或者本地编译：

```bash
cargo build --release          # target/release/ipv6-anchor，约 1.8 MB
```

## 用法

```bash
# token 在 dynv6 控制台：My Zones → 点进主机名 → Details → HTTP Tokens
export DYNV6_TOKEN=你的token

ipv6-anchor --hostname myhost.dynv6.net --dry-run   # 只打印 URL
ipv6-anchor --hostname myhost.dynv6.net             # 常驻，每 300 秒一次
ipv6-anchor --hostname myhost.dynv6.net --once      # 跑一次就退出
```

配置文件（可选）：`cp config.example.toml ipv6-anchor.toml`，只认四个键
`hostname` / `token` / `interval` / `update_prefix`。
查找顺序：`./ipv6-anchor.toml` → `~/.config/ipv6-anchor/config.toml`，也可 `-c 路径`。

优先级：命令行 > 环境变量（`DYNV6_HOSTNAME`、`DYNV6_TOKEN`、`IPV6_ANCHOR_INTERVAL`）> 配置文件。

cron 示例：

```
*/5 * * * * DYNV6_TOKEN=xxx /usr/local/bin/ipv6-anchor --once --hostname myhost.dynv6.net
```

## 日志长什么样

默认（`INFO`）每次检查打几行：结论、连接细节、下次时间。

```
2026-10-27T22:42:22+08:00  INFO  启动：zbz-ai.dns.army → https://ipv6.dynv6.com（每 300s 检查一次，token Q4LX…(30 字符)）
2026-10-27T22:42:22+08:00  INFO  本机出口：2409:8a5c:2a9:3d40:4c1f:9a2e:11b7:33d0 → 2a01:4f9:c010:95b::
2026-10-27T22:42:24+08:00  INFO  已更新：addresses updated
2026-10-27T22:42:24+08:00  INFO  2409:8a5c:2a9:3d40:4c1f:9a2e:11b7:33d0 → 2a01:4f9:c010:95b::  ·  HTTP 200  ·  1.23s
2026-10-27T22:42:24+08:00  INFO  下次检查 22:47:24（300s 后）
```

- **本机出口**：内核为「去 dynv6」选中的源地址，也就是 dynv6 会写进 AAAA 的那个值。做法是 UDP `connect`（只查路由表、不发包）再读 `local_addr()`。链路上有 NAT66 时 dynv6 记的是 NAT 后的地址，这行就不代表实际值了。
- **耗时**：含 DNS + TCP + TLS + 读响应体，正常几百毫秒到两三秒；持续变大说明到 dynv6 的链路在变差。
- **`-v` 追加**：被忽略的代理变量、完整 URL（token 已打码）、出口地址、每次重试各自的失败原因。

## 开机自启（systemd）

仓库里带了一份可直接用的单元：[`contrib/ipv6-anchor.service`](contrib/ipv6-anchor.service)。

```bash
sudo install -Dm 755 target/release/ipv6-anchor /opt/ipv6-anchor/ipv6-anchor
sudo install -Dm 644 contrib/ipv6-anchor.service /etc/systemd/system/
sudo install -Dm 600 contrib/ipv6-anchor.env.example /etc/ipv6-anchor.env
sudo editor /etc/ipv6-anchor.env        # 填 hostname 和 token

sudo systemctl daemon-reload
sudo systemctl enable --now ipv6-anchor
journalctl -u ipv6-anchor -f            # 看日志
```

几个容易踩的点：

- 单元里 `User=` / `WorkingDirectory=` / `ExecStart=` 三处路径要跟机器一致，示例按 `/opt/ipv6-anchor` 写。
- token 放 `EnvironmentFile`（600 权限），别写进 unit——unit 文件一般 644，谁都能看。
- **不要同时开 cron**：程序自己就是常驻循环，再加 cron 只会两边抢着更新。
- 二进制在 `~/opt` 这种家目录下、又不想用 root 的，可以走用户级 systemd：单元放 `~/.config/systemd/user/`，
  `WorkingDirectory=%h/opt/ipv6-anchor`，然后 `systemctl --user enable --now ipv6-anchor`，
  并 `sudo loginctl enable-linger $USER`（否则没登录时不会启动）。
- 开机那一刻 IPv6 的 SLAAC 地址可能还没就绪，第一轮失败没关系——程序每 300s 会自己再来一次。

## 开机自启（macOS）

macOS 没有 systemd，对应的是 **launchd**。三种形态，按需求挑一个：

| 形态 | 什么时候起来 | 要 root | 适合 |
| --- | --- | --- | --- |
| LaunchAgent（`~/Library/LaunchAgents`） | **登录后** | 不要 | 日常在用、会登录的机器（推荐） |
| LaunchDaemon（`/Library/LaunchDaemons`） | **开机后**，没人登录也跑 | 要 | 家里常开、重启后停在登录界面的机器 |
| `.app` + 登录项 | 登录后 | 不要 | 想双击运行 / 签 Developer ID 分发 |

三者的二进制是同一个，区别只在「谁在什么时候把它拉起来」。

### 一键装（LaunchAgent，推荐）

```bash
./contrib/macos/install.sh --hostname myhost.dynv6.net --token 你的token
```

脚本干的事：`cargo build --release` → ad-hoc 签名 → 装到 `~/.local/bin/ipv6-anchor` →
配置写到 `~/.config/ipv6-anchor/config.toml`（600）→ 生成并加载
`~/Library/LaunchAgents/com.rusthing.ipv6-anchor.plist`。全程不需要 sudo。

```bash
launchctl print gui/$(id -u)/com.rusthing.ipv6-anchor   # 状态
tail -f ~/Library/Logs/ipv6-anchor.log                  # 日志
launchctl kickstart -k gui/$(id -u)/com.rusthing.ipv6-anchor  # 重启
./contrib/macos/uninstall.sh [--purge]                  # 卸载
```

不想用脚本就照 plist 模板手抄一遍：
[`contrib/macos/com.rusthing.ipv6-anchor.plist.in`](contrib/macos/com.rusthing.ipv6-anchor.plist.in)
里的 `@BIN@` / `@CONFIG@` / `@LOG@` 换成绝对路径，丢进 `~/Library/LaunchAgents/`，然后

```bash
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.rusthing.ipv6-anchor.plist
```

### 开机即启（LaunchDaemon）

```bash
cargo build --release                                  # sudo 不继承 PATH，先自己编
sudo ./contrib/macos/install.sh --system --skip-build \
     --hostname myhost.dynv6.net --token 你的token
```

配置落在 `/Library/Application Support/ipv6-anchor/config.toml`，日志 `/var/log/ipv6-anchor.log`。
**root 环境没有 `HOME`**，所以 plist 里必须用 `-c 绝对路径` 指配置——脚本已经这么写了，
手改的话别漏。卸载：`sudo ./contrib/macos/uninstall.sh --system --purge`。

### 打成 .app

```bash
./contrib/macos/make-app.sh              # 本机架构 → dist/ipv6-anchor.app
./contrib/macos/make-app.sh --universal  # Intel + Apple Silicon 合一
```

`LSUIElement = true`，没有 Dock 图标、没有窗口，纯后台跑。两个限制：

- 从 Finder / 登录项启动拿不到命令行参数，只能靠 `~/.config/ipv6-anchor/config.toml`；
- 它和 LaunchAgent 是**二选一**，同时开就是两个进程抢着更新同一条 AAAA。

### macOS 专有的坑

- **签名**：`Cargo.toml` 里 `strip = true` 会把链接器给 arm64 打的 ad-hoc 签名弄坏，症状是
  运行即被杀。所以装之前统一 `codesign --force --sign - 二进制`（脚本已含）。自己编自己用，
  ad-hoc 就够，不需要开发者证书。
- **Gatekeeper**：本机编译没有 quarantine 属性，直接能跑；从别处下载来的要
  `xattr -d com.apple.quarantine 文件`，或者正经签 Developer ID + 公证
  （`codesign --options runtime --timestamp --sign "Developer ID Application: …"` →
  `xcrun notarytool submit --wait` → `xcrun stapler staple`）。
- **隐私地址**：见上面「关于 macOS 的隐私地址」。想固定就
  `sudo sysctl -w net.inet6.ip6.use_tempaddr=0`，写进 `/etc/sysctl.conf` 才能重启后保留
  （新系统上不保证生效，功能本身不受影响）。
- **`Bootstrap failed: 5: Input/output error`**：plist 没问题的话，基本是当前 shell 不在
  Aqua 会话里（ssh、IDE 集成终端、某些 agent 拉起的 shell 都会这样）。换 **Terminal.app**
  重跑同一条 `launchctl bootstrap` 即可。
- 睡眠唤醒后进程不会重启，循环接着跑；醒来那一轮会立刻补一次，`interval` 不用调。
- 别同时配 cron / 别的 LaunchAgent，一个常驻循环就够了。

## 三个使用前提

| 前提 | 说明 |
| --- | --- |
| 机器本身有公网 IPv6 | `auto` 靠的就是这条连接的源地址；连不通 IPv6 就没有可锚定的地址 |
| 请求要直连 | 别设 `HTTPS_PROXY`，别接在只做 v4 的代理/NAT 后面 |
| 源地址最好稳定 | 见下 |

**关于 macOS 的隐私地址**：系统默认用会轮换的临时地址（RFC 4941）发起出站连接，`auto` 记录的就会是它——功能上一直可用（轮换后下一轮检查自动跟上），但地址每天变。想锚定稳定地址就关掉隐私地址扩展：

```bash
sudo sysctl -w net.inet6.ip6.use_tempaddr=0
```

Linux / OpenWrt 上 `use_tempaddr` 默认多为 0，一般不用管。

## 排错

| 现象 | 原因 |
| --- | --- |
| `HTTP 401（invalid authentication token）` | token 不对，或不是这个 zone 的 |
| `HTTP 404` | hostname 拼错 / 该 zone 不属于这个 token |
| `已试 3 次` + 连接错误 | 本机没有 IPv6 出口，或被防火墙挡了出站 |
| `io: unexpected end of file` | TCP 通了但 TLS 握手被对端掐断：本机 IPv6 能出但到 dynv6 的路径有问题（常见是 MTU 黑洞 / 中间设备重置），或 `HTTP(S)_PROXY` 指向的代理把域名 REJECT 了。先按下面「自检」跑一遍 |
| 日志 `无变化：addresses unchanged` | 正常，dynv6 侧地址没变 |

自检三连（在跑 ipv6-anchor 的那台机器上执行）：

```bash
env | grep -i proxy                                   # 有输出说明代理变量在干扰
curl -6 -sv --noproxy '*' https://ipv6.dynv6.com/api/update  2>&1 | head -25
curl -6 -s  --noproxy '*' https://ipv6.dynv6.com/api/update          # 期望 401 invalid authentication token
```

`curl` 通而程序不通 → 代理变量问题；两者都不通 → IPv6 链路问题（试 `ping6`/`traceroute6` 到 `ipv6.dynv6.com`，或临时降 MTU 验证）。

退出码：`0` 成功（含"地址没变"）、`1` 请求失败、`2` 配置错误。

## 结构

```
src/
├── main.rs     入口、常驻循环
├── config.rs   CLI / 环境变量 / TOML 合并与校验
├── dynv6.rs    构造 URL、请求、响应判定、重试、出口地址探测
└── logger.rs   带时间戳的日志

contrib/
├── ipv6-anchor.service        systemd 单元
├── ipv6-anchor.env.example    EnvironmentFile 模板
└── macos/
    ├── install.sh                              编译 + 签名 + 装二进制 + 注册自启
    ├── uninstall.sh                            卸载
    ├── make-app.sh                             打成 dist/ipv6-anchor.app
    ├── com.rusthing.ipv6-anchor.plist.in       LaunchAgent 模板（登录后启动）
    └── com.rusthing.ipv6-anchor.daemon.plist.in  LaunchDaemon 模板（开机即启）
```

dynv6 接口文档：<https://dynv6.com/docs/apis>

## License

MIT
