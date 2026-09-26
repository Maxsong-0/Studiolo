//! 仅创建（`create-only`）、独立隔离的 M5 接入方验收宿主；调用方使用真实的 `agent` 二进制。
//! 不伪造也不启用任何主人 HTTP 会话、生产策略、模型密钥或 `Runner`。
use serde_json::json;
use std::{
    fs,
    io::Write,
    net::Ipv4Addr,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::Path,
    sync::Arc,
    time::Duration,
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_core::VaultPath;
use studiolo_host_config::Config;
use studiolo_local_server::{
    CanonicalWorkbench, PreparedUnixIngress,
    board::{BoardPlane, private_board_router},
};
use studiolo_policy::{DesiredInput, PolicyKind, PolicyStore};
use studiolo_supply::AgentStore;
use studiolo_vault_broker::{FsVaultBroker, Mutation, VaultWriter};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 1 {
        return Err("usage: consumer-smoke NEW_ABSOLUTE_DIRECTORY".into());
    }
    let requested = Path::new(&args[0]);
    if !requested.is_absolute() {
        return Err("new absolute smoke directory required".into());
    }
    let root = requested
        .parent()
        .ok_or("parent missing")?
        .canonicalize()?
        .join(requested.file_name().ok_or("name missing")?);
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    for name in [
        "broker", "auth", "audit", "assets", "config", "board", "agent", "policy", "run", "clients",
    ] {
        fs::DirBuilder::new().mode(0o700).create(root.join(name))?;
    }
    fs::DirBuilder::new()
        .mode(0o700)
        .create(root.join("agent/audit"))?;
    let mut broker = FsVaultBroker::open(&root.join("vault"), &root.join("broker"))?;
    for (path, content) in [
        (
            "项目/Studiolo/00-总览/INDEX.md",
            "# Studiolo 消费端实测\n\n此库为独立验收库，内容不来自真实项目。\n",
        ),
        (
            "项目/Studiolo/04-协调/TASKS.md",
            "# 人工协调\n\n- 保留这条人工行：当前 Codex 正在实测。\n",
        ),
        (
            "项目/Studiolo/05-验收/README.md",
            "# 实测产物\n\n当前 Codex 仅通过 MCP 写入此目录。\n",
        ),
        (
            "项目/Other/00-总览/INDEX.md",
            "# Other\n\n允许读取，禁止 AI 写入。\n",
        ),
    ] {
        broker.mutate(Mutation::create(
            VaultPath::parse(path)?,
            content.as_bytes().to_vec(),
            format!("consumer-smoke-bootstrap:{path}"),
        ))?;
    }
    let now = jiff::Timestamp::now().to_string();
    let identities = AgentStore::open(&root.join("agent"))?;
    identities.upgrade_child_instances()?;
    let agent_audit = SqliteAuditLedger::open(&root.join("agent/audit"))?;
    let policy = PolicyStore::open(&root.join("policy"))?;
    for (kind, writable) in [("codex", true), ("claude", true), ("grok", false)] {
        let issued = identities.issue_type(kind, &now)?;
        private_file(
            &root.join(format!("clients/{kind}.token")),
            issued.token.as_bytes(),
        )?;
        let activation = policy.bootstrap(DesiredInput {
            kind: PolicyKind::CapabilitySet,
            subject: kind.into(),
            body: Some(json!({"ai_type":kind,
                "read":{"project_whitelist":["项目/Studiolo","项目/Other"],"deny_by_default":true},
                "write":{"board":writable,"board_projects":if writable{vec!["项目/Studiolo"]}else{vec![]},"content_patch_only":true,"content_direct":false},
                "policy":{"propose_desired":true,"activate":false},"hard_bound_skills":[]}).to_string()),
        }, &now)?;
        agent_audit.append(studiolo_policy::audit::bootstrapped(&activation)?)?;
    }
    let audit = Arc::new(SqliteAuditLedger::open(&root.join("audit"))?);
    let config = root.join("config/owner.json");
    private_file(
        &config,
        &serde_json::to_vec(&json!({
            "version":1,"auth_directory":root.join("auth"),"audit_directory":root.join("audit"),
            "owner":{"issuer":"https://consumer-smoke.invalid","audience":"isolated-consumer-smoke",
                "subject":"consumer-smoke-owner","email":"consumer-smoke@example.invalid"},
            "idle_seconds":600,"absolute_seconds":1800,"seed_ref":"consumer-smoke-no-owner-session"
        }))?,
    )?;
    let roots = Config::load(&config)?.canonical_serving_roots(
        &root.join("vault"),
        &root.join("assets"),
        &root.join("broker"),
    )?;
    let owner =
        CanonicalWorkbench::prepare_with_audit(broker, Arc::new(roots), audit.clone()).await?;
    let dirs = [
        "vault", "broker", "assets", "auth", "audit", "config", "agent", "policy",
    ]
    .map(|name| root.join(name));
    let isolated = dirs.iter().map(|p| p.as_path()).collect::<Vec<_>>();
    let plane = match BoardPlane::open(
        owner.clone(),
        &root.join("board"),
        &root.join("agent"),
        &root.join("policy"),
        &isolated,
        audit.clone(),
    ) {
        Ok(plane) => plane,
        Err(error) => {
            owner.shutdown().await?;
            return Err(error.into());
        }
    };
    let socket = root.join("run/board.sock");
    let bound = PreparedUnixIngress::prepare(&socket, &isolated)?.bind()?;
    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    let maintenance = tokio::spawn(plane.clone().maintain_until(stopped.clone()));
    let app = private_board_router(plane.clone());
    let serving = tokio::spawn(async move {
        bound
            .serve_shutdown(app, async move {
                let _ = stopped.changed().await;
            })
            .await
    });
    // agent 端口由本宿主持有到退出（T-171）：占位带 SO_REUSEADDR 绑在 0.0.0.0、不监听，人手启动的
    // agent 经 tokio 绑 127.0.0.1 的同一端口；期间同机别的进程的自动分配选不到它。旧写法取号后立即释放，
    // 等人启动 agent 时端口可能已被占用。语义说明见 `tests/loopback_port/mod.rs`。
    let reservation = tokio::net::TcpSocket::new_v4()?;
    reservation.set_reuseaddr(true)?;
    reservation.bind((Ipv4Addr::UNSPECIFIED, 0).into())?;
    let port = reservation.local_addr()?.port();
    let descriptor = json!({"mode":"isolated-consumer-smoke","root":root,
        "mcp":format!("http://127.0.0.1:{port}/mcp"),"policy_hash":policy.effective()?.effective_hash,
        "agent_argv":["serve","--vault",root.join("vault"),"--data",root.join("agent"),
            "--policy",root.join("policy"),"--board-socket",socket,"--allow","项目/Studiolo",
            "--allow","项目/Other","--port",port.to_string()]});
    private_file(
        &root.join("connection.json"),
        &serde_json::to_vec_pretty(&descriptor)?,
    )?;
    println!("{descriptor}");
    println!(
        "Isolated board ready; start the real studiolo-agent with agent_argv. Ctrl-C stops this host; 30-minute ceiling."
    );
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = tokio::time::sleep(Duration::from_secs(1800)) => {},
    }
    drop(reservation);
    stop.send(true)?;
    serving.await??;
    maintenance.await??;
    drop(plane);
    owner.shutdown().await?;
    println!(
        "{}",
        json!({"stopped":true,"canonical_audit_events":audit.query(&AuditFilter::default())?.len(),"agent_audit_events":agent_audit.query(&AuditFilter::default())?.len(),"data_retained":root})
    );
    Ok(())
}
