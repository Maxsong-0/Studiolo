//! `studiolo-agent` 的库部分：装配、门禁、MCP/HTTP 只读外壳与管理子命令；二进制只做参数分发。
//!
//! 只服务接入方的只读供给（M3 首片）；不持有 canonical 写能力，不读配置文件，不创建目录。

pub mod admin;
pub mod auth;
pub mod board_client;
mod http_text;
pub mod index;
mod mcp_transport;
pub mod options;
pub mod refresh;
pub mod serve;
pub mod server;
pub mod state;

/// 按命令行参数运行一个命令。
pub async fn run(args: &[String]) -> Result<(), String> {
    match options::parse(args)? {
        options::Command::Help => {
            println!("{}", options::USAGE);
            Ok(())
        }
        options::Command::Serve(options) => serve::run(options).await,
        options::Command::Admin(command) => admin::run(command),
    }
}
