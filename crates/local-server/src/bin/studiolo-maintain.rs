//! 只进入本机事务维护，不装配 HTTP、应用会话或自动初始化。

use std::io::Write;

fn main() -> std::process::ExitCode {
    match studiolo_local_server::maintenance::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
