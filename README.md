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
2. **不走环境代理**——ureq 默认不读 `HTTP(S)_PROXY`，这里保持默认。一旦请求经代理，dynv6 看到的是代理 IP，`auto` 就废了。

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
| 日志 `无变化：addresses unchanged` | 正常，dynv6 侧地址没变 |

退出码：`0` 成功（含"地址没变"）、`1` 请求失败、`2` 配置错误。

## 结构

```
src/
├── main.rs     入口、常驻循环
├── config.rs   CLI / 环境变量 / TOML 合并与校验
├── dynv6.rs    构造 URL、请求、响应判定、重试
└── logger.rs   带时间戳的日志
```

dynv6 接口文档：<https://dynv6.com/docs/apis>

## License

MIT
