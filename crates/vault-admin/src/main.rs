//! 首建命令的进程边界；无 HTTP、无模型或因子认证能力。

use std::io::Write;
use studiolo_vault_admin::{Command, initialize, parse, plan};

fn run() -> Result<(), String> {
    // 管理命令也必须固定私有创建权限，不能依赖调用 shell 的 umask。
    rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));
    let residency = studiolo_process_safety::harden().map_err(|error| error.to_string())?;
    writeln!(std::io::stderr(), "秘密驻留面：{residency}")
        .map_err(|_| "无法记录进程加固分类；尚未读取配置".to_owned())?;
    let parsed = parse(std::env::args_os().skip(1).take(142).collect())
        .map_err(|error| error.to_string())?;
    let output = match parsed {
        None => "studiolo-vault-admin plan --config ABS --vault ABS --writer-data ABS --assets ABS --source-snapshot ABS --allow 项目/Studiolo [--allow _规则]\nstudiolo-vault-admin initialize <同一组参数> --expected-plan SHA256\n源必须为操作者冻结的私有快照；两次扫描不是跨文件原子快照。目标须已存在且为空。失败后保留现场，禁止自动重试或清理。".into(),
        Some((command, options)) => match command {
            Command::Plan => serde_json::to_string(&plan(&options).map_err(|e| e.to_string())?),
            Command::Initialize { expected_plan } => serde_json::to_string(
                &initialize(&options, &expected_plan).map_err(|e| e.to_string())?,
            ),
        }.map_err(|_| "结果序列化失败；操作可能已提交，请先核对现状".to_owned())?,
    };
    writeln!(std::io::stdout(), "{output}")
        .map_err(|_| "结果输出失败；操作可能已提交，禁止自动重试".to_owned())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
