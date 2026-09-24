//! `studiolo-sync-launcher serve`（root 系统服务）与 `studiolo-sync-launcher payload`（垫片）。
//!
//! 参数闭集：只认这两个子命令且不带其它参数；其余一律退出码 2 并打印固定中文。
//! `serve` 读固定配置、建私有 socket 并循环应答（`serve.rs`）；`payload` 是 transient unit
//! 内的垫片，从 stdin 的控制 FIFO 取启动事实、报回 (dev,ino)、等放行后 exec（`payload.rs`）。

use std::io::Write;
use std::process::ExitCode;
use studiolo_sync_launcher::{payload, serve};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let command = args.next();
    if args.next().is_some() {
        return refuse("参数闭集：只接受 serve 或 payload，且不带其它参数");
    }
    match command.as_deref().and_then(|value| value.to_str()) {
        Some("serve") => serve::run(),
        Some("payload") => payload::run(),
        _ => refuse("用法：studiolo-sync-launcher serve | payload"),
    }
}

fn refuse(message: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "{message}");
    ExitCode::from(2)
}
