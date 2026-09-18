//! 可信本地首次登记装配；不启动HTTP、子进程，不签发人类身份或会话。

mod environment;
mod options;
mod terminal;
mod workflow;

use std::io::Write;
use studiolo_host_config::Config;

#[derive(Debug, thiserror::Error)]
pub(crate) enum AdminError {
    #[error("{0}")]
    Process(#[from] studiolo_process_safety::Refusal),
    #[error("无法记录秘密驻留面逐项分类；未读取秘密")]
    ProcessReport,
    #[error("参数无效；仅接受 --help 中的非秘密参数")]
    Arguments,
    #[error("非秘密配置或私有路径检查失败；要求明确配置、0700目录与0600配置文件")]
    Config,
    #[error("专用种子环境缺失或不规范；需要40位小写十六进制，禁止在命令参数中提供秘密")]
    Environment,
    #[error("审计库不可用或不安全；不登记、不自动修复")]
    Audit,
    #[error("审计库不存在；先核对目录，再显式执行 migrate")]
    MissingAudit,
    #[error("{0}")]
    Session(#[from] studiolo_auth::SessionError),
    #[error("{0}")]
    Terminal(#[from] terminal::TerminalError),
    #[error("结果输出失败；操作可能已提交，请修复输出后核对现状，勿自动重试")]
    Output,
}

impl From<studiolo_host_config::ConfigError> for AdminError {
    fn from(_: studiolo_host_config::ConfigError) -> Self {
        Self::Config
    }
}

fn run() -> Result<(), AdminError> {
    // 先收紧秘密驻留面，再把逐项分类记录到标准错误：本平台不提供的能力必须被
    // 操作者看见，只在内部返回而不记录等于静默跳过。摘要只含措施名与固定分类，
    // 永不含种子、验证码或任何环境值；记录失败即终止，且此时尚未读取任何秘密。
    let residency = studiolo_process_safety::harden()?;
    writeln!(std::io::stderr(), "秘密驻留面：{residency}")
        .map_err(|_| AdminError::ProcessReport)?;
    // 限制参数数量；拒绝时绝不回显任何未知参数原文。
    let args = std::env::args_os().skip(1).take(7).collect();
    let Some(options) = options::parse(args)? else {
        writeln!(
            std::io::stdout(),
            "可信本地首次TOTP登记（不提供网络登录或丢机恢复）\n\
            studiolo-auth-admin enroll --config /绝对路径/owner.json\n\
            studiolo-auth-admin audit-flush --config /绝对路径/owner.json\n\
            studiolo-auth-admin migrate --config /绝对路径/owner.json --from 1|2|3\n\
            配置及存储目录须预先批准；迁移才允许初始化固定库，普通登记不创建库。\n\
            种子仅由获批机制注入 STUDIOLO_TOTP_SEED_HEX；不要放在参数、配置或日志中。"
        )
        .map_err(|_| AdminError::Output)?;
        return Ok(());
    };
    let config = Config::load(&options.config)?;
    terminal::preflight()?;
    let seed = match options.command {
        options::Command::Migrate(_) => None,
        _ => Some(environment::read_seed(config.seed_ref().to_owned())?),
    };
    let report = workflow::execute(options.command, config, seed, terminal::read_code)?;
    writeln!(std::io::stdout(), "{report}").map_err(|_| AdminError::Output)
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(
                std::io::stderr(),
                "{error}\n操作可能已提交或验证码可能已消费；补记不激活。修复后先核对现状，禁止自动重试或清库重登。"
            );
            std::process::ExitCode::FAILURE
        }
    }
}
