//! 有界分表重开读取；先计数和长度，再分配元数据。
use super::{
    BridgeError, STATE_MAX,
    database::{error, revision},
    dbwrite::total,
    input_codec,
    model::*,
    state_codec,
};
use rusqlite::Connection;
pub(super) fn load(db: &Connection) -> Result<State> {
    let count: i64 = db
        .query_row("SELECT count(*) FROM bridge_meta", [], |r| r.get(0))
        .map_err(error)?;
    if count != 1 {
        return Err(BridgeError::CorruptStore);
    }
    // 在分配任何 payload/ack 前验证全库实际元数据总量，不信任仅声明的计数器。
    let counts: (i64, i64) = db
        .query_row(
            "SELECT (SELECT count(*) FROM bridge_plans),(SELECT count(*) FROM bridge_actions)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(error)?;
    if counts.0 > 3 || counts.1 > 8192 {
        return Err(BridgeError::Budget);
    }
    let malformed:i64=db.query_row("SELECT (SELECT count(*) FROM bridge_meta WHERE typeof(revision)!='integer' OR typeof(encoded_total)!='integer' OR typeof(payload)!='blob')+(SELECT count(*) FROM bridge_plans WHERE typeof(sequence)!='integer' OR typeof(role)!='text' OR role NOT IN ('pending','last','fuse') OR typeof(input)!='blob' OR length(input) NOT BETWEEN 1 AND 16777216 OR (ack IS NOT NULL AND (typeof(ack)!='text' OR length(CAST(ack AS BLOB))>8192)))+(SELECT count(*) FROM bridge_actions WHERE typeof(sequence)!='integer' OR typeof(position)!='integer' OR position NOT BETWEEN 0 AND 4095 OR typeof(payload)!='blob' OR length(payload) NOT BETWEEN 1 AND 32768)",[],|r|r.get(0)).map_err(error)?;
    let roles: i64 = db
        .query_row("SELECT count(DISTINCT role) FROM bridge_plans", [], |r| {
            r.get(0)
        })
        .map_err(error)?;
    if malformed != 0 || roles != counts.0 {
        return Err(BridgeError::CorruptStore);
    }
    let declared: i64 = db
        .query_row("SELECT encoded_total FROM bridge_meta", [], |r| r.get(0))
        .map_err(error)?;
    let actual = total(db)?;
    if actual < 0 || declared < 0 || actual != declared || actual > STATE_MAX as i64 {
        return Err(BridgeError::Budget);
    }
    let meta:Option<Vec<u8>>=db.query_row("SELECT CASE WHEN length(payload) BETWEEN 1 AND 67108864 THEN payload END FROM bridge_meta WHERE id=1",[],|r|r.get(0)).map_err(error)?;
    let mut state = state_codec::decode(&meta.ok_or(BridgeError::Budget)?)?;
    if state.version != 0 {
        return Err(BridgeError::CorruptStore);
    }
    state.version = revision(db)?;
    if state.pending.is_some() || state.last.is_some() || state.fuse.is_some() {
        return Err(BridgeError::CorruptStore);
    }
    let mut rows=db.prepare("SELECT sequence,CASE role WHEN 'pending' THEN 'pending' WHEN 'last' THEN 'last' WHEN 'fuse' THEN 'fuse' ELSE '' END,CASE WHEN length(input) BETWEEN 1 AND 16777216 THEN input END,CASE WHEN ack IS NULL OR length(CAST(ack AS BLOB))<=8192 THEN ack ELSE '' END FROM bridge_plans ORDER BY sequence").map_err(error)?;
    let mut records = rows.query([]).map_err(error)?;
    while let Some(row) = records.next().map_err(error)? {
        let sequence: i64 = row.get(0).map_err(error)?;
        let role: String = row.get(1).map_err(error)?;
        let bytes: Option<Vec<u8>> = row.get(2).map_err(error)?;
        let ack: Option<String> = row.get(3).map_err(error)?;
        let input = input_codec::decode(&bytes.ok_or(BridgeError::Budget)?)?;
        if sequence <= 0 || input.plan_sequence != sequence as u64 {
            return Err(BridgeError::CorruptStore);
        }
        let actions = load_actions(db, sequence)?;
        if role == "fuse" {
            if !actions.is_empty() {
                return Err(BridgeError::CorruptStore);
            }
            state.fuse = Some(Fuse { input, ack });
            continue;
        }
        let crate::Decision::Ready(cached_plan) =
            crate::plan(input.clone(), None).map_err(|_| BridgeError::CorruptStore)?
        else {
            return Err(BridgeError::CorruptStore);
        };
        let pending = Pending::new(input, cached_plan, actions, ack)?;
        match role.as_str() {
            "pending" => state.pending = Some(pending),
            "last" => state.last = Some(pending),
            _ => return Err(BridgeError::CorruptStore),
        }
    }
    let stored: i64 = db
        .query_row("SELECT encoded_total FROM bridge_meta", [], |r| r.get(0))
        .map_err(error)?;
    if stored != total(db)? || stored > STATE_MAX as i64 {
        return Err(BridgeError::CorruptStore);
    }
    let orphan:i64=db.query_row("SELECT count(*) FROM bridge_actions a LEFT JOIN bridge_plans p USING(sequence) WHERE p.sequence IS NULL",[],|r|r.get(0)).map_err(error)?;
    if orphan != 0 {
        return Err(BridgeError::CorruptStore);
    }
    super::validate::state(&state)?;
    Ok(state)
}
fn load_actions(db: &Connection, sequence: i64) -> Result<Vec<Progress>> {
    let mut query=db.prepare("SELECT position,CASE WHEN length(payload) BETWEEN 1 AND 32768 THEN payload END FROM bridge_actions WHERE sequence=?1 ORDER BY position").map_err(error)?;
    let mut rows = query.query([sequence]).map_err(error)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(error)? {
        let i: i64 = row.get(0).map_err(error)?;
        let bytes: Option<Vec<u8>> = row.get(1).map_err(error)?;
        if i != out.len() as i64 || out.len() >= 4096 {
            return Err(BridgeError::CorruptStore);
        }
        out.push(state_codec::decode_progress(
            &bytes.ok_or(BridgeError::Budget)?,
        )?);
    }
    Ok(out)
}
