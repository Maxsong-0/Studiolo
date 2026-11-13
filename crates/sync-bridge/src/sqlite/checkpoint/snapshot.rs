//! 当前全量模型验证与规范基线编码复用生产逻辑，避免备份旁路接受损坏状态。
use super::super::{
    RECEIPT_MAX, STATE_MAX, audit, codec::Writer, database, input_codec, model::State, receipts,
};
use crate::Binding;
use studiolo_audit_ledger::{AuditRecord, FrozenAudit, ReceiptCoverage};
use studiolo_checkpoint_storage::Error;
use studiolo_core::checkpoint::OwnerWatermark;
pub(super) fn load(db: &rusqlite::Connection, expected: &Binding) -> Result<State, Error> {
    database::sealed_schema(db).map_err(invalid)?;
    database::integrity(db).map_err(invalid)?;
    let state = database::load(db).map_err(invalid)?;
    if &state.binding != expected
        || state.pending.is_some()
        || state.retirement.is_some()
        || audit::next(&state).map_err(invalid)?.is_some()
    {
        return Err(Error::Invalid);
    }
    Ok(state)
}
pub(super) fn watermark(state: &State) -> Result<OwnerWatermark, Error> {
    let mut writer = Writer::new(STATE_MAX);
    writer
        .raw(b"bridge-checkpoint-baseline-v1\0")
        .map_err(invalid)?;
    input_codec::baseline(&mut writer, &state.baseline).map_err(invalid)?;
    Ok(OwnerWatermark::Bridge {
        link_id: receipts::hex(&state.binding.link_id),
        scope_digest: receipts::hex(&state.binding.scope_hash),
        common_base_digest: receipts::hex(&receipts::hash(&writer.bytes)),
        observation_sequence: state.cursor.last_canonical_observation,
        durable_gate_closed: true,
    })
}
pub(super) fn verify_audit(state: &State, audit: &FrozenAudit) -> Result<ReceiptCoverage, Error> {
    let mut coverage = ReceiptCoverage::default();
    let mut visit = |encoded: &str| -> Result<(), Error> {
        if encoded.len() > RECEIPT_MAX {
            return Err(Error::Invalid);
        }
        let record: AuditRecord = serde_json::from_str(encoded).map_err(invalid)?;
        coverage
            .include(audit.verify_receipt(&record).map_err(invalid)?)
            .map_err(invalid)
    };
    if let Some(last) = &state.last {
        for action in &last.actions {
            for ack in [&action.intent_ack, &action.outcome_ack]
                .into_iter()
                .flatten()
            {
                visit(ack)?;
            }
        }
        visit(last.summary_ack.as_deref().ok_or(Error::Invalid)?)?;
    }
    if let Some(fuse) = &state.fuse {
        visit(fuse.ack.as_deref().ok_or(Error::Invalid)?)?;
    }
    Ok(coverage)
}
fn invalid<T>(_: T) -> Error {
    Error::Invalid
}
