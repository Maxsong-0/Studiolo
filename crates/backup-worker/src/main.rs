//! 独立备份 worker 的进程入口：三个固定子命令，没有别的参数，也不读环境变量。
//!
//! - `run`：常驻循环，由服务管理器以 worker 账号拉起；
//! - `schedule`：一次性命令，由定时器拉起，请主服务判断该不该做一次封存；
//! - `init`：显式初始化 worker 的状态目录，只在首次部署或人工重置时手动跑。

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    use std::process::ExitCode;
    use studiolo_backup::worker::audit::QueueAnswer;
    use studiolo_backup_worker::serve;

    let mut arguments = std::env::args_os().skip(1);
    let (Some(command), None) = (arguments.next(), arguments.next()) else {
        eprintln!("用法：studiolo-backup-worker run｜schedule｜init");
        return ExitCode::from(2);
    };
    let failed = match command.to_str() {
        Some("run") => serve::run(),
        Some("schedule") => match serve::schedule() {
            Ok(answer) => {
                // 三种答复都算这次调用成功：做不做是主服务按批准的间隔定的。
                println!(
                    "{}",
                    match answer {
                        QueueAnswer::Started => "started",
                        QueueAnswer::Outstanding => "outstanding",
                        _ => "not_due",
                    }
                );
                return ExitCode::SUCCESS;
            }
            Err(error) => error,
        },
        Some("init") => match serve::initialize() {
            Ok(()) => return ExitCode::SUCCESS,
            Err(error) => error,
        },
        _ => {
            eprintln!("用法：studiolo-backup-worker run｜schedule｜init");
            return ExitCode::from(2);
        }
    };
    eprintln!("{failed}");
    ExitCode::FAILURE
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("studiolo-backup-worker 只在 Linux 上运行");
    std::process::ExitCode::from(2)
}
