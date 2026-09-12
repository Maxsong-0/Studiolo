//! 显式选择loopback或私有Unix入口；不同认证模式不相互降级。

mod options;
mod startup;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    // Unix非UTF8参数不能触发默认panic诊断并回显原字节；参数数量也有固定预算。
    let args = std::env::args_os()
        .skip(1)
        .take(24)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "参数必须是规范UTF8；不回显原值".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if args.len() > 22 {
        return Err("启动参数超过固定预算".into());
    }
    let Some(options) = options::parse(args.into_iter())? else {
        println!(
            "Studiolo只读工作台\n\n\
            用法：studiolo-local-server --vault <副本目录> [--assets web/dist] [--label 只读副本] [--port 4317]\n\n\
            旧公共入口同时指定：\n  --public-origin <https://公共域名>\n  --access-team <https://团队名.cloudflareaccess.com>\n  --access-audience <Access应用audience>\n  --access-email <允许访问的邮箱>\n\n\
            显式M2会话入口：\n  --session-config </绝对路径/owner.json>\n  --public-origin <https://公共域名>\n\
            此模式的Vault/assets也须绝对路径，与私有配置/双库隔离；不得混用旧--access-*参数。\n\
            复用登记工具的非秘密配置及已登记v3/v4既有库；普通会话模式不建库、不迁移、不读种子。\n\n\
            可加 --canonical 读取已初始化的canonical及权威revision；仅会话模式可用，仍不开放写入。\n\n\
            或使用 --canonical-writer-data </绝对路径/既有Broker数据目录> 启用完整捕获与监视新鲜度；与--canonical互斥。\n\
            新模式持canonical独占租约，不得同时启动M0写者；仅open_existing，不初始化或猜用Auth/Audit目录。\n\
            HTTP仍只读，Broker可能补已确定的完成回执和水位，因此不承诺进程零写。\n\n\
            显式因子入口：--factor-socket </绝对路径/私有代理目录/http.sock>\n\
            可选 --replacement-seed-ref <预配置候选引用>；必须配合会话配置/公共Origin，不得指定--port。\n\
            仅因子模式读取固定STUDIOLO_TOTP_SEED_HEX与可选STUDIOLO_TOTP_REPLACEMENT_SEED_HEX环境槽；不得把秘密写进参数或配置。\n\
            因子模式要求已登记v4及私有Unix入口；其余只监听127.0.0.1。初始化失败直接退出，不回退。\n\
            请先构建web，再显式指定副本；不会自动打开真实Obsidian库。"
        );
        println!(
            "\n图2首片可选：--project-repository <项目/名称=/绝对仓库路径>\n\
             仅本机/旧Access只读模式支持一个显式绑定；默认只枚举目录。\n\
             可加 --cargo-dependencies 显式读取根及成员Cargo.toml，派生清单依赖；不执行Cargo或读取源码。\n\
             或加 --rust-call-sites '[\"src/lib.rs\"]' 显式读取Rust文件白名单，展示调用语法位置；与Cargo能力独立。\n\
             源码白名单不自动跟随mod/include，不执行代码或宏，不解析调用目标。\n\
             文档代次与仓库观测时间分别标识；未绑定项目显示未核验。"
        );
        return Ok(());
    };
    startup::PreparedServer::prepare(options)
        .await
        .map_err(|error| error.to_string())?
        .serve()
        .await
        .map_err(|error| error.to_string())
}
