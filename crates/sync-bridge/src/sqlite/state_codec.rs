//! 有界控制状态/动作 wire 编码；分表保存，SQLite 不保存用户正文。

use super::{
    BridgeError, INPUT_MAX, MAX_PATHS, RECEIPT_MAX, STATE_MAX,
    codec::{Reader, Writer, json},
    input_codec as ic,
    model::*,
};

pub(crate) fn encode(s: &State) -> Result<Vec<u8>> {
    let mut w = Writer::new(STATE_MAX);
    w.raw(b"BRSTATE1")?;
    w.num(s.version)?;
    ic::binding(&mut w, &s.binding)?;
    ic::limits(&mut w, s.limits)?;
    ic::cursor(&mut w, &s.cursor)?;
    ic::baseline(&mut w, &s.baseline)?;
    w.option(&s.pending, pending)?;
    w.option(&s.last, pending)?;
    w.num(s.floor)?;
    w.option(&s.retirement, |w, v| {
        w.num(v.old_sequence)?;
        w.raw(&v.old_digest)?;
        w.num(v.old_floor)?;
        ack(w, &v.ack)
    })?;
    w.option(&s.fuse, |w, v| {
        w.blob(&ic::encode(&v.input)?, INPUT_MAX)?;
        ack(w, &v.ack)
    })?;
    Ok(w.bytes)
}
pub(crate) fn decode(bytes: &[u8]) -> Result<State> {
    let mut r = Reader::new(bytes)?;
    if r.raw(8)? != b"BRSTATE1" {
        return Err(BridgeError::UnsupportedSchema);
    }
    let state = State {
        version: r.num()?,
        binding: ic::read_binding(&mut r)?,
        limits: ic::read_limits(&mut r)?,
        cursor: ic::read_cursor(&mut r)?,
        baseline: ic::read_baseline(&mut r)?,
        pending: r.option(read_pending)?,
        last: r.option(read_pending)?,
        floor: r.num()?,
        retirement: r.option(|r| {
            Ok(Retirement {
                old_sequence: r.num()?,
                old_digest: r.fixed()?,
                old_floor: r.num()?,
                ack: read_ack(r)?,
            })
        })?,
        fuse: r.option(|r| {
            Ok(Fuse {
                input: ic::decode(r.blob(INPUT_MAX)?)?,
                ack: read_ack(r)?,
            })
        })?,
    };
    r.end()?;
    if encode(&state)? != bytes {
        return Err(BridgeError::CorruptStore);
    }
    Ok(state)
}
pub(crate) fn pending_bytes(p: &Pending) -> Result<Vec<u8>> {
    let mut w = Writer::new(STATE_MAX);
    pending(&mut w, p)?;
    Ok(w.bytes)
}
fn pending(w: &mut Writer, p: &Pending) -> Result<()> {
    w.blob(&ic::encode(&p.input)?, INPUT_MAX)?;
    if p.actions.len() > MAX_PATHS {
        return Err(BridgeError::Budget);
    }
    w.num(p.actions.len() as u64)?;
    for a in &p.actions {
        progress(w, a)?;
    }
    ack(w, &p.summary_ack)
}
fn progress(w: &mut Writer, a: &Progress) -> Result<()> {
    w.text(&a.key, 256)?;
    w.byte(u8::from(a.issued))?;
    w.option(&a.receipt, |w, v| {
        w.text(&json(v, RECEIPT_MAX)?, RECEIPT_MAX)
    })?;
    ack(w, &a.intent_ack)?;
    ack(w, &a.outcome_ack)?;
    Ok(())
}
pub(crate) fn progress_bytes(a: &Progress) -> Result<Vec<u8>> {
    let mut w = Writer::new(32768);
    progress(&mut w, a)?;
    Ok(w.bytes)
}
pub(crate) fn decode_progress(bytes: &[u8]) -> Result<Progress> {
    if bytes.len() > 32768 {
        return Err(BridgeError::Budget);
    }
    let mut r = Reader::new(bytes)?;
    let p = read_progress(&mut r)?;
    r.end()?;
    if progress_bytes(&p)? != bytes {
        return Err(BridgeError::CorruptStore);
    }
    Ok(p)
}
fn read_pending(r: &mut Reader<'_>) -> Result<Pending> {
    let input = ic::decode(r.blob(INPUT_MAX)?)?;
    let mut actions = Vec::new();
    for _ in 0..r.count(MAX_PATHS)? {
        actions.push(read_progress(r)?);
    }
    let crate::Decision::Ready(cached_plan) =
        crate::plan(input.clone(), None).map_err(|_| BridgeError::CorruptStore)?
    else {
        return Err(BridgeError::CorruptStore);
    };
    Pending::new(input, cached_plan, actions, read_ack(r)?)
}
fn read_progress(r: &mut Reader<'_>) -> Result<Progress> {
    let key = r.text(256)?;
    let issued = match r.byte()? {
        0 => false,
        1 => true,
        _ => return Err(BridgeError::CorruptStore),
    };
    let receipt = r.option(|r| {
        let value = r.text(RECEIPT_MAX)?;
        let decoded: EffectReceipt =
            serde_json::from_str(&value).map_err(|_| BridgeError::CorruptStore)?;
        if json(&decoded, RECEIPT_MAX)? != value {
            return Err(BridgeError::CorruptStore);
        }
        Ok(decoded)
    })?;
    Ok(Progress {
        key,
        issued,
        receipt,
        intent_ack: read_ack(r)?,
        outcome_ack: read_ack(r)?,
    })
}
fn ack(w: &mut Writer, v: &Option<String>) -> Result<()> {
    w.option(v, |w, v| w.text(v, RECEIPT_MAX))
}
fn read_ack(r: &mut Reader<'_>) -> Result<Option<String>> {
    r.option(|r| r.text(RECEIPT_MAX))
}
