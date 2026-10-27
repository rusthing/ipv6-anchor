//! dynv6 Update API 客户端。
//!
//! 只发一个请求：
//!
//! ```text
//! GET https://ipv6.dynv6.com/api/update?hostname=…&ipv6=auto&token=…
//! ```
//!
//! `ipv6=auto` 让 dynv6 直接用「这条连接的源 IPv6 地址」写 AAAA，
//! 所以本机不需要探测地址，也不需要回显服务；地址有没有变由 dynv6 自己判断
//! （返回 `addresses updated` / `addresses unchanged`），本地也不用存状态。
//!
//! 「这条连接一定走 IPv6」由端点域名保证：`ipv6.dynv6.com` 只有 AAAA 记录。
//! 不用 `IpFamily::Ipv6Only` 强制——ureq 3 在 macOS 上那样会直接 `host not found`。

use anyhow::{anyhow, Result};
use std::fmt;
use std::time::Duration;

use crate::config::Settings;
use crate::logger;

const MAX_ATTEMPTS: u32 = 3;

/// 会被 ureq 自动读取的代理环境变量。
///
/// ureq 3.5 的 `Config::default()` 里写死了 `proxy: Proxy::try_from_env()`，
/// 已经没有 `proxy-from-env` 这个特性开关了——光靠 `ureq = "3"` 是关不掉的。
/// 走代理的后果不是「请求失败」而是「静默写错」：dynv6 的 `ipv6=auto` 记下的是
/// 代理出口的地址，本机的 AAAA 永远不更新。所以必须显式 `.proxy(None)`。
const PROXY_ENV_VARS: [&str; 6] = [
    "ALL_PROXY",
    "all_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
];

/// 当前环境里设置了哪些代理变量（只用于日志提示，不参与决策）
fn proxy_env_vars_present() -> Vec<&'static str> {
    PROXY_ENV_VARS
        .iter()
        .copied()
        .filter(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Updated(String),
    Unchanged(String),
}

impl Outcome {
    pub fn changed(&self) -> bool {
        matches!(self, Outcome::Updated(_))
    }

    pub fn message(&self) -> &str {
        match self {
            Outcome::Updated(m) | Outcome::Unchanged(m) => m,
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag = if self.changed() { "已更新" } else { "无变化" };
        write!(f, "{tag}：{}", self.message())
    }
}

/// 非 2xx。4xx 重试也没用，直接报错退出本轮。
#[derive(Debug)]
struct StatusError {
    status: u16,
    body: String,
}

impl fmt::Display for StatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.body.is_empty() {
            write!(f, "HTTP {}", self.status)
        } else {
            write!(f, "HTTP {}（{}）", self.status, self.body)
        }
    }
}

impl std::error::Error for StatusError {}

pub fn build_url(s: &Settings) -> String {
    let mut url = format!(
        "{}/api/update?hostname={}&ipv6=auto",
        s.endpoint,
        enc(&s.hostname)
    );
    if s.update_prefix {
        url.push_str("&ipv6prefix=auto");
    }
    url.push_str(&format!("&token={}", enc(&s.token)));
    url
}

/// 把 token 从 URL 里换掉，只给日志看
pub fn redact(url: &str, token: &str) -> String {
    if token.is_empty() {
        url.to_string()
    } else {
        url.replace(token, "<token>")
    }
}

pub fn update(s: &Settings) -> Result<Outcome> {
    let proxies = proxy_env_vars_present();
    if !proxies.is_empty() {
        logger::debug(format!(
            "已忽略代理环境变量：{}（必须直连，否则 AAAA 会写成代理出口地址）",
            proxies.join(", ")
        ));
    }

    let agent = ureq::Agent::config_builder()
        // 非 2xx 也当正常响应，好把 dynv6 的错误正文读出来
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(concat!("ipv6-anchor/", env!("CARGO_PKG_VERSION")))
        // 覆盖 Config::default() 里的 Proxy::try_from_env()，强制直连
        .proxy(None)
        .build()
        .into();

    let url = build_url(s);
    logger::debug(format!("GET {}", redact(&url, &s.token)));

    let mut last: Option<anyhow::Error> = None;
    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            let backoff = Duration::from_millis(500 * 2u64.pow(attempt - 1));
            logger::debug(format!("{backoff:?} 后重试"));
            std::thread::sleep(backoff);
        }
        match once(&agent, &url) {
            Ok(o) => return Ok(o),
            Err(e) => {
                logger::debug(format!("第 {} 次失败：{e:#}", attempt + 1));
                if is_fatal(&e) {
                    return Err(e);
                }
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("未知错误")).context(format!(
        "已试 {MAX_ATTEMPTS} 次"
    )))
}

/// 把 ureq 的错误翻成人话。
///
/// TLS 握手阶段被对端掐断时，ureq 只会吐出 `io: unexpected end of file`，
/// 光看这句分不清是代理拦截、IPv6 链路不通还是对端故障，所以补一句提示。
fn describe(e: &ureq::Error) -> String {
    if let ureq::Error::Io(io) = e {
        if io.kind() == std::io::ErrorKind::UnexpectedEof {
            return format!(
                "{io}（连接在 TLS 握手阶段被对端关闭：常见于代理拦截 / 无 IPv6 出口 / 中间设备重置）"
            );
        }
    }
    format!("{e}")
}

fn once(agent: &ureq::Agent, url: &str) -> Result<Outcome> {
    let mut resp = agent.get(url).call().map_err(|e| anyhow!("{}", describe(&e)))?;
    let status = resp.status().as_u16();
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow!("读取响应失败：{e}"))?;
    let text = body.trim().to_string();

    if !(200..300).contains(&status) {
        return Err(StatusError { status, body: text }.into());
    }
    Ok(classify(&text))
}

/// dynv6 用纯文本回话：`addresses updated` / `addresses unchanged`
fn classify(body: &str) -> Outcome {
    let lower = body.to_ascii_lowercase();
    let unchanged = lower.contains("unchanged")
        || lower.contains("no change")
        || lower.contains("not changed");
    if unchanged {
        Outcome::Unchanged(fallback(body, "addresses unchanged"))
    } else {
        Outcome::Updated(fallback(body, "addresses updated"))
    }
}

fn fallback(body: &str, default: &str) -> String {
    if body.is_empty() {
        default.to_string()
    } else {
        body.chars().take(200).collect()
    }
}

fn is_fatal(e: &anyhow::Error) -> bool {
    match e.downcast_ref::<StatusError>() {
        Some(s) => (400..500).contains(&s.status) && s.status != 408 && s.status != 429,
        None => false,
    }
}

/// 百分号编码，够用
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Cli;
    use clap::Parser;

    fn settings(args: &[&str]) -> Settings {
        let cli = Cli::parse_from(std::iter::once("ipv6-anchor").chain(args.iter().copied()));
        Settings::resolve(&cli).unwrap()
    }

    #[test]
    fn url_has_auto_and_token_last() {
        let s = settings(&["--hostname", "h.dynv6.net", "--token", "tok"]);
        assert_eq!(
            build_url(&s),
            "https://ipv6.dynv6.com/api/update?hostname=h.dynv6.net&ipv6=auto&token=tok"
        );
    }

    #[test]
    fn url_with_prefix() {
        let s = settings(&["--hostname", "h.dynv6.net", "--token", "tok", "--update-prefix"]);
        assert!(build_url(&s).contains("&ipv6prefix=auto&token=tok"));
    }

    #[test]
    fn redacts_token() {
        let s = settings(&["--hostname", "h.dynv6.net", "--token", "secret123"]);
        let u = redact(&build_url(&s), &s.token);
        assert!(u.contains("<token>"));
        assert!(!u.contains("secret123"));
    }

    #[test]
    fn classify_responses() {
        assert!(classify("addresses updated").changed());
        assert!(!classify("addresses unchanged").changed());
        assert!(!classify("Addresses Unchanged").changed());
        assert!(classify("").changed());
        assert_eq!(classify("addresses updated").message(), "addresses updated");
    }

    #[test]
    fn encoding() {
        assert_eq!(enc("a.b-c_d"), "a.b-c_d");
        assert_eq!(enc("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn eof_error_gets_a_hint() {
        let e = ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        let msg = describe(&e);
        assert!(msg.contains("unexpected end of file"), "{msg}");
        assert!(msg.contains("TLS 握手阶段"), "{msg}");
    }

    #[test]
    fn other_io_error_passes_through() {
        let e = ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
        assert!(!describe(&e).contains("TLS 握手阶段"));
    }
}
