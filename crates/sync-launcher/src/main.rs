//! `studiolo-sync-launcher serve`（root 系统服务）与 `studiolo-sync-launcher payload`（垫片）。
//!
//! 参数闭集：只认这两个子命令且不带其它参数；其余一律退出码 2 并打印固定中文。
//! 真实实现（socket 循环、systemd-run、FIFO、statfs）由 B 轮 W2 在 `serve.rs`／`payload.rs`／
//! `unit.rs` 落地；本入口在那之前对两个子命令都返回固定错误，不启动任何东西。

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let command = args.next();
    if args.next().is_some() {
        return refuse("参数闭集：只接受 serve 或 payload，且不带其它参数");
    }
    match command.as_deref().and_then(|value| value.to_str()) {
        Some("serve") => {
            // B 轮 W2 实现：serve::run() —— 读 config → 建 socket（0750，SO_PEERCRED 核 owner uid）
            // → 循环 decode_request → session 状态机 → 应答。
            refuse("serve 尚未接入：可信 launcher 的服务循环由 B 轮 W2 实现")
        }
        Some("payload") => {
            // B 轮 W2 实现：payload::run() —— 从 LISTEN_FDNAMES 取 release／report 两个 fd，
            // stat 四个对象报回，阻塞读放行 FIFO，env_clear 只加 XDG_CONFIG_HOME／LANG／TZ，exec 封闭 argv。
            refuse("payload 尚未接入：垫片由 B 轮 W2 实现")
        }
        _ => refuse("用法：studiolo-sync-launcher serve | payload"),
    }
}

fn refuse(message: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "{message}");
    ExitCode::from(2)
}
