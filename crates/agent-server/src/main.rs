//! `studiolo-agent`：AI 接入方的 MCP 主外壳（Streamable HTTP，仅 loopback）与可信本地管理子命令。

use std::io::Write as _;

fn main() -> std::process::ExitCode {
    // 运行时手工建，收尾才能设上限：`#[tokio::main]` 的运行时析构会无限期等阻塞线程上的重扫或副本校验，
    // 停服可能因此拖过 systemd 的 TimeoutStopSec 而被强杀（T-184，BUG-033）。
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        let _ = writeln!(std::io::stderr(), "无法建立异步运行时");
        return std::process::ExitCode::FAILURE;
    };
    let code = runtime.block_on(entry());
    runtime.shutdown_timeout(studiolo_agent_server::shutdown::RUNTIME_LIMIT);
    code
}

async fn entry() -> std::process::ExitCode {
    // 先收紧秘密驻留面，再做任何别的事。本进程持有两处明文：`admin type-issue` 铸出的
    // 32 字节类型 token，以及每次请求头里的 bearer —— 前者是 MCP 供给面的全部身份。
    // 崩溃转储、在线进程检视与换出到交换区这三条路径对它们一视同仁，**与执行哪个子命令
    // 无关**（`--help` 与失败的管理命令同样收紧），所以位置在参数解析之前。
    //
    // 与另外三个持秘密的程序（`studiolo-auth-admin` / `studiolo-local-server` /
    // `studiolo-maintain`）共用 `studiolo-process-safety` 这一份实现与同一套失败语义：
    // 本平台声明支持的能力必须真正生效并回读确认，否则**整体拒绝启动**且不读取任何秘密，
    // 不降级成「尽力而为、失败就继续」；本平台不提供的能力记为明确的「不支持」。
    //
    // 逐项分类随即记录到标准错误——只返回不记录，等于把本平台的「不支持」藏起来，与静默
    // 跳过没有区别。走标准错误而不是标准输出，是因为标准输出留给管理子命令的非秘密结果
    // （允许重定向与管道）；摘要只含措施名与固定分类，永不含 token 或任何环境值。
    //
    // 这是本程序最早的无条件路径：`main` 只是在此之前建好运行时，其间既不读取
    // 秘密也不派生子进程；本 crate 的 `src/` 由架构守卫禁用 `Command::new`，因此不存在
    // 更早的派生窗口可以把未清除的环境槽位带出去。
    let residency = match studiolo_process_safety::harden() {
        Ok(residency) => residency,
        Err(refusal) => {
            let _ = writeln!(std::io::stderr(), "{refusal}");
            return std::process::ExitCode::FAILURE;
        }
    };
    if writeln!(std::io::stderr(), "秘密驻留面：{residency}").is_err() {
        // 记录写不出去，就等于把逐项分类藏了起来；此时也无从报错，只能以失败退出码终止，
        // 绝不带着一份没人看得见的加固结论继续跑。
        return std::process::ExitCode::FAILURE;
    }
    // Unix 非 UTF-8 参数不回显原值；参数数量有固定预算。
    // 预算随白名单规模上调（T-101）：每个白名单项占两个参数（--allow 与值），
    // 24 项即 48 个，加上 replica-root/max-age/data/index-cache/port/policy 等约 13 个。
    // 80 可容纳约 33 个白名单项；上限本身保留，用于挡住异常长的参数表。
    let args = match std::env::args_os()
        .skip(1)
        .take(82)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "参数必须是 UTF-8；不回显原值".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(args) if args.len() <= 80 => args,
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
