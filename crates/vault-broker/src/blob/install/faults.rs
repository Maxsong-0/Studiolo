//! 仅测试的精确阶段漂移；生产没有回调、环境变量或跳过校验入口。
use crate::mutation::record::Operation;
use crate::writer::directory;
use crate::{BrokerError, FsVaultBroker};
use std::sync::atomic::Ordering;

pub(crate) fn before_history_check_test(
    broker: &FsVaultBroker,
    op: &Operation,
    installing: bool,
) -> Result<(), BrokerError> {
    let first = if installing { 82 } else { 80 };
    for (code, revision) in [
        (first, op.previous_revision),
        (first + 1, Some(op.target_revision)),
    ] {
        if broker
            .writer
            .fault
            .compare_exchange(code, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let revision = revision.ok_or(BrokerError::CorruptWriterState)?;
            directory(&broker.data, "history")?
                .remove_file(revision.to_string())
                .map_err(|e| BrokerError::io("inject L0 drift before installation", e))?;
        }
    }
    Ok(())
}
