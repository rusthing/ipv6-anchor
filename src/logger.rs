//! 极简日志：时间戳 + 级别，默认写 stderr。
//!
//! 级别：0 = 仅错误，1 = 信息（默认），2 = 调试（`-v`）。

use std::sync::atomic::{AtomicU8, Ordering};

static LEVEL: AtomicU8 = AtomicU8::new(1);

pub fn init(verbose: bool) {
    LEVEL.store(if verbose { 2 } else { 1 }, Ordering::Relaxed);
}

fn emit(level: &str, msg: &str) {
    let ts = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z");
    eprintln!("{ts}  {level:<5} {msg}");
}

pub fn error(msg: impl AsRef<str>) {
    emit("ERROR", msg.as_ref());
}

pub fn info(msg: impl AsRef<str>) {
    emit("INFO", msg.as_ref());
}

pub fn debug(msg: impl AsRef<str>) {
    if LEVEL.load(Ordering::Relaxed) >= 2 {
        emit("DEBUG", msg.as_ref());
    }
}
