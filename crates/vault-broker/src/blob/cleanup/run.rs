//! 清理不创建 canonical 终态；先重验全批，删除前再次核对根、日志、目录和完整字节。
use super::{BlobPruneReport, Plan, check, child, scan};
use crate::{BrokerError, FsVaultBroker};
use std::path::Path;

pub(super) fn validate(plan: &Plan) -> Result<(), BrokerError> {
    for domain in &plan.domains {
        if scan::inventory(&domain.root)? != domain.inventory {
            return Err(BrokerError::CaptureChanged);
        }
        for (path, proof) in &domain.files {
            child::check(&domain.root, path, &proof.node, true)?;
        }
    }
    Ok(())
}

pub(super) fn execute(
    broker: &FsVaultBroker,
    mut plan: Plan,
) -> Result<BlobPruneReport, BrokerError> {
    check(broker, &plan)?;
    validate(&plan)?;
    let mut report = BlobPruneReport {
        removed: 0,
        bytes: 0,
    };
    for index in 0..plan.domains.len() {
        for file_index in 0..plan.domains[index].files.len() {
            check(broker, &plan)?;
            // 另一固定域也不得在本批执行中漂移；自身已删除项的目录快照逐次更新。
            for domain in &plan.domains {
                if scan::inventory(&domain.root)? != domain.inventory {
                    return Err(BrokerError::CaptureChanged);
                }
            }
            let domain = &mut plan.domains[index];
            let (path, expected) = &domain.files[file_index];
            let actual = child::scan(&domain.root, path, &expected.node.file)?;
            if actual.node.file != expected.node.file || actual.identity != expected.identity {
                return Err(BrokerError::CaptureChanged);
            }
            if scan::inventory(&domain.root)? != domain.inventory {
                return Err(BrokerError::CaptureChanged);
            }
            super::admission(broker)?;
            if super::journal(broker)? != plan.journal {
                return Err(BrokerError::CaptureChanged);
            }
            child::check(&domain.root, path, &expected.node, false)?;
            crate::mutation::fault(broker, 40)?;
            domain
                .root
                .remove_file(path.as_str())
                .map_err(|e| BrokerError::io("prune unpublished blob temporary", e))?;
            FsVaultBroker::sync_directory(&domain.root, Path::new("."))?;
            report.removed += 1;
            report.bytes += expected.identity.bytes;
            let position = domain
                .inventory
                .names
                .binary_search_by(|name| name.as_str().cmp(path.as_str()))
                .map_err(|_| BrokerError::CaptureChanged)?;
            domain.inventory.names.remove(position);
            let observed = scan::inventory(&domain.root)?;
            if observed.names != domain.inventory.names
                || observed.stamp.identity != domain.inventory.stamp.identity
            {
                return Err(BrokerError::CaptureChanged);
            }
            domain.inventory = observed;
            // 仅测试的一次性中断：已删除部分不能被报告为全批成功。
            crate::mutation::fault(broker, 41)?;
        }
    }
    check(broker, &plan)?;
    for domain in &plan.domains {
        if scan::inventory(&domain.root)? != domain.inventory {
            return Err(BrokerError::CaptureChanged);
        }
    }
    Ok(report)
}
