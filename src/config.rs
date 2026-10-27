//! 配置：命令行 > 环境变量 > 配置文件 > 默认值。
//!
//! 只有主机名和 token 是必需的，其余都有合理默认值。

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::logger;

/// dynv6 的 IPv6 专用入口：域名只解析 AAAA，
/// 从传输层就保证请求走 IPv6，`ipv6=auto` 才有意义。
pub const DEFAULT_ENDPOINT: &str = "https://ipv6.dynv6.com";

#[derive(Debug, Parser)]
#[command(
    name = "ipv6-anchor",
    version,
    about = "用 dynv6 更新本机的 IPv6 DDNS 记录",
    long_about = None
)]
pub struct Cli {
    /// 配置文件路径（默认 ./ipv6-anchor.toml，其次 ~/.config/ipv6-anchor/config.toml）
    #[arg(long, short = 'c', value_name = "PATH", env = "IPV6_ANCHOR_CONFIG")]
    pub config: Option<PathBuf>,

    /// dynv6 主机名，例如 myhost.dynv6.net
    #[arg(long, value_name = "HOST", env = "DYNV6_HOSTNAME")]
    pub hostname: Option<String>,

    /// dynv6 HTTP token
    #[arg(long, value_name = "TOKEN", env = "DYNV6_TOKEN")]
    pub token: Option<String>,

    /// 检查间隔（秒），默认 300
    #[arg(long, value_name = "SECS", env = "IPV6_ANCHOR_INTERVAL")]
    pub interval: Option<u64>,

    /// 顺便把 zone 的 IPv6 前缀也更新掉（路由器会跟着换前缀时用）
    #[arg(long)]
    pub update_prefix: bool,

    /// 只跑一次就退出（cron / 计划任务用）
    #[arg(long)]
    pub once: bool,

    /// 只打印请求 URL，不真的发
    #[arg(long)]
    pub dry_run: bool,

    /// 输出调试日志
    #[arg(long, short = 'v')]
    pub verbose: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct FileConfig {
    hostname: Option<String>,
    token: Option<String>,
    interval: Option<u64>,
    update_prefix: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub hostname: String,
    pub token: String,
    pub interval: Duration,
    pub update_prefix: bool,
    pub endpoint: String,
    pub once: bool,
    pub dry_run: bool,
}

impl Settings {
    pub fn resolve(cli: &Cli) -> Result<Self> {
        let (config_path, file) = load_file(cli)?;

        let mut s = Settings {
            hostname: String::new(),
            token: String::new(),
            interval: Duration::from_secs(300),
            update_prefix: false,
            endpoint: DEFAULT_ENDPOINT.to_string(),
            once: cli.once,
            dry_run: cli.dry_run,
        };

        if let Some(f) = &file {
            s.hostname = f.hostname.clone().unwrap_or(s.hostname);
            s.token = f.token.clone().unwrap_or(s.token);
            s.interval = f.interval.map(Duration::from_secs).unwrap_or(s.interval);
            s.update_prefix = f.update_prefix.unwrap_or(s.update_prefix);
        }

        // 命令行（clap 已经把同名环境变量读进来了）覆盖文件
        if let Some(v) = &cli.hostname {
            s.hostname = v.clone();
        }
        if let Some(v) = &cli.token {
            s.token = v.clone();
        }
        if let Some(v) = cli.interval {
            s.interval = Duration::from_secs(v);
        }
        if cli.update_prefix {
            s.update_prefix = true;
        }
        // 仅用于调试/自测：指向 mock 服务
        if let Ok(v) = env::var("IPV6_ANCHOR_ENDPOINT") {
            s.endpoint = v;
        }

        s.hostname = s
            .hostname
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        s.endpoint = s.endpoint.trim().trim_end_matches('/').to_string();
        if s.endpoint.is_empty() {
            s.endpoint = DEFAULT_ENDPOINT.to_string();
        }

        if s.hostname.is_empty() {
            bail!("缺少 hostname：--hostname / DYNV6_HOSTNAME / 配置文件里写 hostname");
        }
        if !s.hostname.contains('.') || s.hostname.contains('/') || s.hostname.contains(char::is_whitespace)
        {
            bail!("hostname 应该是完整域名，例如 myhost.dynv6.net，而不是 {}", s.hostname);
        }
        if s.token.is_empty() && !s.dry_run {
            bail!("缺少 token：--token / DYNV6_TOKEN / 配置文件里写 token");
        }
        if s.interval < Duration::from_secs(60) {
            logger::debug("interval 小于 60 秒，已按 60 秒处理");
            s.interval = Duration::from_secs(60);
        }

        if let Some(p) = &config_path {
            logger::debug(format!("配置文件：{}", p.display()));
        }
        Ok(s)
    }

    /// 日志里不露 token
    pub fn masked_token(&self) -> String {
        let n = self.token.chars().count();
        if n == 0 {
            "(未设置)".into()
        } else if n <= 6 {
            "*".repeat(n)
        } else {
            format!("{}…({n} 字符)", self.token.chars().take(4).collect::<String>())
        }
    }
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn load_file(cli: &Cli) -> Result<(Option<PathBuf>, Option<FileConfig>)> {
    let explicit = cli.config.clone();

    let candidates: Vec<PathBuf> = match &explicit {
        Some(p) => vec![expand_tilde(p)],
        None => vec![
            PathBuf::from("ipv6-anchor.toml"),
            home_dir().join(".config/ipv6-anchor/config.toml"),
        ],
    };

    for path in candidates {
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("读取配置文件失败：{}", path.display()))?;
        let cfg: FileConfig = toml::from_str(&text)
            .with_context(|| format!("解析配置文件失败：{}", path.display()))?;
        return Ok((Some(path), Some(cfg)));
    }

    if let Some(p) = explicit {
        bail!("配置文件不存在：{}", p.display());
    }
    Ok((None, None))
}

fn expand_tilde(p: &Path) -> PathBuf {
    match p.to_string_lossy().strip_prefix("~/") {
        Some(rest) => home_dir().join(rest),
        None => p.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn cli(args: &[&str]) -> Cli {
        Cli::parse_from(std::iter::once("ipv6-anchor").chain(args.iter().copied()))
    }

    #[test]
    fn minimal_config() {
        let s = Settings::resolve(&cli(&["--hostname", "MyHost.dynv6.net.", "--token", "t"])).unwrap();
        assert_eq!(s.hostname, "myhost.dynv6.net");
        assert_eq!(s.endpoint, "https://ipv6.dynv6.com");
        assert_eq!(s.interval, Duration::from_secs(300));
        assert!(!s.update_prefix);
    }

    #[test]
    fn interval_floor() {
        let s = Settings::resolve(&cli(&["--hostname", "a.b", "--token", "t", "--interval", "5"])).unwrap();
        assert_eq!(s.interval, Duration::from_secs(60));
    }

    #[test]
    fn hostname_required() {
        let e = Settings::resolve(&cli(&["--token", "t"])).unwrap_err();
        assert!(e.to_string().contains("hostname"));
    }

    #[test]
    fn hostname_must_be_fqdn() {
        let e = Settings::resolve(&cli(&["--hostname", "myhost", "--token", "t"])).unwrap_err();
        assert!(e.to_string().contains("完整域名"));
    }

    #[test]
    fn token_required_unless_dry_run() {
        assert!(Settings::resolve(&cli(&["--hostname", "a.b"])).is_err());
        assert!(Settings::resolve(&cli(&["--hostname", "a.b", "--dry-run"])).is_ok());
    }

    #[test]
    fn token_is_masked() {
        let s = Settings::resolve(&cli(&["--hostname", "a.b", "--token", "abcdef0123456789"])).unwrap();
        let m = s.masked_token();
        assert!(m.starts_with("abcd"));
        assert!(!m.contains("ef0123456789"));
    }

    #[test]
    fn unknown_key_rejected() {
        let e: Result<FileConfig> = toml::from_str("hostname = \"a.b\"\ntypo = 1").map_err(Into::into);
        assert!(e.is_err());
    }
}
