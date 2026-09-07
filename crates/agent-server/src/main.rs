//! `studiolo-agent`：AI 接入方的 MCP 主外壳（Streamable HTTP，仅 loopback）与可信本地管理子命令。

#[tokio::main]
async fn main() -> std::process::ExitCode {
    // Unix 非 UTF-8 参数不回显原值；参数数量有固定预算。
    let args = match std::env::args_os()
        .skip(1)
        .take(40)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "参数必须是 UTF-8；不回显原值".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(args) if args.len() <= 38 => args,
        Ok(_) => {
            eprintln!("参数数量超过固定预算");
            return std::process::ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    match studiolo_agent_server::run(&args).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
