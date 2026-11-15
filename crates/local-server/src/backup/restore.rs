//! 把已解包的封存恢复进新根：Broker 重绑定，然后重开控制库和两条审计链。
//! 可选 owner 必须显式缺席或给出目录；给出目录但打不开则失败，不把缺库当成未启用。
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_control_db::ControlDb;
use studiolo_core::VaultUuid;
use studiolo_policy::PolicyStore;
use studiolo_vault_broker::{NewRootRebind, RestoreReceipt, rebind_new_roots};

/// 可选 owner 的显式状态。不能省略字段来表示缺席。
#[derive(Clone, Copy)]
pub enum OwnerPresence<'a> {
    /// 这次封存没有该 owner。
    Absent,
    /// 新根里的该 owner 目录，必须能重开。
    Present(&'a std::path::Path),
}

/// 一次新根恢复要打开的全部位置。
pub struct RestoreRequest<'a> {
    /// Broker 的新 canonical/data 根和期望身份。
    pub broker: NewRootRebind<'a>,
    /// 含 control.sqlite3 的目录。
    pub control_directory: &'a std::path::Path,
    /// 人侧审计链目录。
    pub human_audit: &'a std::path::Path,
    /// 接入方审计链。封存了接入方时必须 Present。
    pub agent_audit: OwnerPresence<'a>,
    /// 策略库目录。
    pub policy: OwnerPresence<'a>,
    /// 接入方身份库目录。
    pub agent_store: OwnerPresence<'a>,
}

/// 重开结果。两个审计链分别记录，不能用一条链代替另一条。
#[derive(Clone, Debug)]
pub struct RestoredRoot {
    /// Broker 重绑定回执。
    pub broker: RestoreReceipt,
    /// 人侧账本已重开。
    pub human_audit: bool,
    /// 接入方账本已重开；缺席为 false。
    pub agent_audit: bool,
    /// 控制库已用重绑定后的 Vault 身份重开。
    pub control: bool,
    /// 策略库已重开；缺席为 false。
    pub policy: bool,
    /// 接入方身份库已重开；缺席为 false。
    pub agent_store: bool,
}

/// 恢复失败。不包含路径。
#[derive(Debug)]
pub enum RestoreFailure {
    /// 新根重绑定没有完成。
    Broker(studiolo_vault_broker::RestoreError),
    /// 人侧或接入方审计链打不开。
    Audit,
    /// 控制库打不开，或 Vault 身份与重绑定结果不一致。
    Control,
    /// 策略库目录给出了但打不开。
    Policy,
    /// 接入方身份库目录给出了但打不开。
    AgentStore,
}

impl std::fmt::Display for RestoreFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("新根恢复未完成")
    }
}
impl std::error::Error for RestoreFailure {}

/// 重绑定 Broker，再重开控制库和两条审计链。可选库只有 Present 才打开。
pub fn restore_into_new_root(request: RestoreRequest<'_>) -> Result<RestoredRoot, RestoreFailure> {
    let broker = rebind_new_roots(&request.broker).map_err(RestoreFailure::Broker)?;
    if broker.vault_uuid != request.broker.expected_vault_uuid {
        return Err(RestoreFailure::Control);
    }
    SqliteAuditLedger::open_existing(request.human_audit).map_err(|_| RestoreFailure::Audit)?;
    let agent_audit = match request.agent_audit {
        OwnerPresence::Absent => false,
        OwnerPresence::Present(path) => {
            SqliteAuditLedger::open_existing(path).map_err(|_| RestoreFailure::Audit)?;
            true
        }
    };
    let control = ControlDb::open(
        request.control_directory.join("control.sqlite3"),
        broker.vault_uuid,
    )
    .map_err(|_| RestoreFailure::Control)?;
    if control
        .bound_vault_uuid()
        .map_err(|_| RestoreFailure::Control)?
        != broker.vault_uuid
    {
        return Err(RestoreFailure::Control);
    }
    drop(control);
    let policy = match request.policy {
        OwnerPresence::Absent => false,
        OwnerPresence::Present(path) => {
            PolicyStore::open_existing(path).map_err(|_| RestoreFailure::Policy)?;
            true
        }
    };
    let agent_store = match request.agent_store {
        OwnerPresence::Absent => false,
        OwnerPresence::Present(path) => {
            studiolo_supply::database::reopen_restored_store(path)
                .map_err(|_| RestoreFailure::AgentStore)?;
            true
        }
    };
    Ok(RestoredRoot {
        broker,
        human_audit: true,
        agent_audit,
        control: true,
        policy,
        agent_store,
    })
}

/// 重绑定后的 Vault 身份。供调用方核对两条账本没有被换成另一个库。
pub fn restored_vault(report: &RestoredRoot) -> VaultUuid {
    report.broker.vault_uuid
}
