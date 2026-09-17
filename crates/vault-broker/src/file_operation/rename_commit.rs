//! 两个可见操作各自先持久 Issued；中途失败保留唯一槽和原键，不补跑或回滚。
use super::{record::Active, *};
use crate::{BrokerError, FsVaultBroker, mutation};
pub(crate) fn execute(
    broker: &FsVaultBroker,
    state: &mut mutation::State,
) -> Result<(), BrokerError> {
    let active = state
        .file_operation
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?;
    rename_prepare::check(broker, active)?;
    let roots = super::store::required(broker)?;
    if crate::blob::file::missing(
        &roots.quarantine,
        &crate::blob::store::logical(&format!("{}.object", active.frozen.operation_id))?,
    )?
    .is_none()
    {
        return Err(BrokerError::CaptureChanged);
    }
    rename_capacity::before_issue(broker)?;
    capacity::free(broker, active.reserved_bytes)?;
    active.phase = FileOperationPhase::TargetInstallIssued;
    mutation::save(broker, state)?;
    let result = (|| {
        mutation::fault(broker, 72)?;
        rename_target::install(broker, selected(state)?)?;
        mutation::save(broker, state)?;
        mutation::fault(broker, 75)?;
        rename_source::prepare(broker, selected(state)?)?;
        selected(state)?.phase = FileOperationPhase::SourceDetachIssued;
        mutation::save(broker, state)?;
        mutation::fault(broker, 76)?;
        rename_source::detach(broker, selected(state)?)?;
        mutation::save(broker, state)?;
        mutation::fault(broker, 79)
    })();
    result.map_err(|_| BrokerError::MutationUnresolved)
}
fn selected(state: &mut mutation::State) -> Result<&mut Active, BrokerError> {
    state
        .file_operation
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)
}
