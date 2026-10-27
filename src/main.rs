//! ipv6-anchor —— 用 dynv6 更新本机的 IPv6 DDNS 记录。
//!
//! 靠 dynv6 的 `ipv6=auto`：地址由 dynv6 从连接源地址里取，本地不探测、不存状态。

mod config;
mod dynv6;
mod logger;

use std::process::ExitCode;
use std::thread;

use anyhow::Result;
use clap::Parser;

use config::{Cli, Settings};

fn main() -> ExitCode {
    let cli = Cli::parse();
    logger::init(cli.verbose);

    let settings = match Settings::resolve(&cli) {
        Ok(s) => s,
        Err(e) => {
            logger::error(format!("配置有误：{e:#}"));
            return ExitCode::from(2);
        }
    };

    match run(&settings) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            logger::error(format!("{e:#}"));
            ExitCode::from(1)
        }
    }
}

fn run(s: &Settings) -> Result<()> {
    if s.dry_run {
        println!(
            "GET {}",
            dynv6::redact(&dynv6::build_url(s), &s.token)
        );
        return Ok(());
    }

    if s.once {
        return tick(s);
    }

    logger::info(format!(
        "启动：{} → {}（每 {}s 检查一次，token {}）",
        s.hostname,
        s.endpoint,
        s.interval.as_secs(),
        s.masked_token()
    ));
    match dynv6::probe_route(&s.endpoint) {
        Some(r) => logger::info(format!("本机出口：{r}")),
        None => logger::info("本机出口：未知（没有可用的 IPv6 路由？）"),
    }

    let mut failures: u32 = 0;
    loop {
        match tick(s) {
            Ok(()) => failures = 0,
            Err(e) => {
                failures += 1;
                logger::error(format!("本轮失败（连续 {failures} 次）：{e:#}"));
            }
        }
        let next = chrono::Local::now() + chrono::Duration::from_std(s.interval).unwrap_or_default();
        logger::info(format!(
            "下次检查 {}（{}s 后）",
            next.format("%H:%M:%S"),
            s.interval.as_secs()
        ));
        thread::sleep(s.interval);
    }
}

fn tick(s: &Settings) -> Result<()> {
    let report = dynv6::update(s)?;
    logger::info(report.outcome.to_string());
    logger::info(report.detail());
    Ok(())
}
