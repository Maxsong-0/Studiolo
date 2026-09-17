//! 只清理两个固定私有域中的未发布 blob；无未决操作时先验整批，之后逐项重验。
mod child;
mod run;
mod scan;
#[cfg(test)]
mod tests;

use super::file::{self, ContentProof};
use crate::{
    BrokerError, FsVaultBroker, VaultPath, WriterPhase,
    writer::{directory, disk},
};
use cap_std::fs::Dir;
use std::sync::Arc;

/// 固定 history/snapshots 临时文件的一次完整整理结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlobPruneReport {
    /// 完整核验后实际删除的临时文件数。
    pub removed: usize,
    /// 实际删除的逻辑字节数，不以磁盘分配块数替代。
    pub bytes: u64,
}

struct Domain {
    root: Dir,
    inventory: scan::Inventory,
    files: Vec<(VaultPath, ContentProof)>,
}

// 不对调用方开放可构造/可重放的删除清单；票据仅属于本次 Broker 实例。
struct Plan {
    instance: Arc<()>,
    journal: ContentProof,
    domains: Vec<Domain>,
}

impl FsVaultBroker {
    /// 仅在 Ready 且无恢复/维护未决时整理自有未发布 blob；不触碰已发布历史、快照或证据。
    /// 每个固定域独立限制 64 个、512 MiB，每个文件至多 256 MiB，并以 64 KiB 分块核验。
    /// 中途 I/O 或身份漂移错误可能发生在部分已核验文件删除后；错误不代表整批成功。
    pub fn prune_blob_temporaries(&mut self) -> Result<BlobPruneReport, BrokerError> {
        let plan = prepare(self)?;
        run::execute(self, plan)
    }
}

fn admission(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    crate::maintenance::ensure_idle(broker)?;
    let status = broker.write_status()?;
    if status.phase != WriterPhase::Ready
        || status.unresolved
        || broker.content_audit_status()?.restore_unconfirmed
    {
        return Err(BrokerError::MutationUnresolved);
    }
    broker.writer.check(&broker.root, &broker.data)
}

fn journal(broker: &FsVaultBroker) -> Result<ContentProof, BrokerError> {
    file::scan(
        &broker.data,
        &logical("writer-state.json")?,
        disk::RECORD_LIMIT as u64,
        None,
    )
}

fn check(broker: &FsVaultBroker, plan: &Plan) -> Result<(), BrokerError> {
    if !Arc::ptr_eq(&broker.capture_instance, &plan.instance) {
        return Err(BrokerError::CaptureChanged);
    }
    admission(broker)?;
    if journal(broker)? != plan.journal {
        return Err(BrokerError::CaptureChanged);
    }
    Ok(())
}

fn prepare(broker: &FsVaultBroker) -> Result<Plan, BrokerError> {
    admission(broker)?;
    let mut plan = Plan {
        instance: broker.capture_instance.clone(),
        journal: journal(broker)?,
        domains: Vec::new(),
    };
    // 根路径是封闭枚举；不递归，也不接受传入的目录或任意 glob。
    for name in ["history", "snapshots"] {
        let root = directory(&broker.data, name)?;
        let inventory = scan::inventory(&root)?;
        let files = scan::candidates(&root, &inventory)?;
        if scan::inventory(&root)? != inventory {
            return Err(BrokerError::CaptureChanged);
        }
        plan.domains.push(Domain {
            root,
            inventory,
            files,
        });
    }
    check(broker, &plan)?;
    run::validate(&plan)?;
    Ok(plan)
}

fn logical(name: &str) -> Result<VaultPath, BrokerError> {
    VaultPath::parse(name).map_err(|_| BrokerError::CorruptWriterState)
}
