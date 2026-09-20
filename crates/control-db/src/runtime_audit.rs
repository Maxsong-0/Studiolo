//! Durable metadata-only audit outcomes are committed with the authority change.
use super::*;

/// Pending non-endpoint runtime fact, recorded atomically with its DB transition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeAuditRecord {
    /// Stable identity for ledger replay.
    pub event_id: String,
    /// Stable action name.
    pub action: String,
    /// Sanitized metadata only; never artifact or note bodies.
    pub payload: Value,
}
pub(super) fn enqueue(
    tx: &Transaction<'_>,
    event_id: &str,
    action: &str,
    payload: Value,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT OR IGNORE INTO runtime_audit_outbox(event_id,action,payload_json) VALUES(?1,?2,?3)",
        params![event_id, action, serde_json::to_string(&payload)?],
    )?;
    Ok(())
}
impl ControlDb {
    /// Lists durable runtime facts whose audit sink has not yet acknowledged them.
    pub fn pending_runtime_audits(&self) -> Result<Vec<RuntimeAuditRecord>, DbError> {
        let mut statement=self.connection.prepare("SELECT event_id,action,payload_json FROM runtime_audit_outbox WHERE delivered=0 ORDER BY sequence")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (event_id, action, payload) = row?;
            events.push(RuntimeAuditRecord {
                event_id,
                action,
                payload: serde_json::from_str(&payload)?,
            });
        }
        Ok(events)
    }
    /// Called only after the authoritative audit append succeeds.
    pub fn acknowledge_runtime_audit(&mut self, event_id: &str) -> Result<(), DbError> {
        self.connection.execute(
            "UPDATE runtime_audit_outbox SET delivered=1 WHERE event_id=?1",
            [event_id],
        )?;
        Ok(())
    }
}
