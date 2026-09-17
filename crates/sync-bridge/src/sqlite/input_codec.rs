//! 纯 DTO 的私有 wire adapter；不要求 planner/types 依赖 serde 或数据库。

use super::{
    BridgeError, INPUT_MAX, MAX_PATHS,
    codec::{Reader, Writer},
    model::Result,
};
use crate::*;
use std::collections::{BTreeMap, BTreeSet};
use studiolo_core::VaultPath;

pub(crate) fn encode(input: &Input) -> Result<Vec<u8>> {
    let mut w = Writer::new(INPUT_MAX);
    w.raw(b"BRINPUT1")?;
    binding(&mut w, &input.scope.binding)?;
    w.num(input.scope.paths.len() as u64)?;
    for value in &input.scope.paths {
        path(&mut w, value)?;
    }
    baseline(&mut w, &input.baseline)?;
    inventory(&mut w, &input.canonical)?;
    inventory(&mut w, &input.mirror)?;
    cursor(&mut w, &input.cursor)?;
    w.num(input.expected_baseline_version)?;
    w.num(input.plan_sequence)?;
    limits(&mut w, input.limits)?;
    Ok(w.bytes)
}
pub(crate) fn decode(bytes: &[u8]) -> Result<Input> {
    if bytes.len() > INPUT_MAX {
        return Err(BridgeError::Budget);
    }
    let mut r = Reader::new(bytes)?;
    if r.raw(8)? != b"BRINPUT1" {
        return Err(BridgeError::UnsupportedSchema);
    }
    let binding = read_binding(&mut r)?;
    let mut paths = BTreeSet::new();
    for _ in 0..r.count(MAX_PATHS)? {
        if !paths.insert(read_path(&mut r)?) {
            return Err(BridgeError::CorruptStore);
        }
    }
    let value = Input {
        scope: ApprovedScope { binding, paths },
        baseline: read_baseline(&mut r)?,
        canonical: read_inventory(&mut r)?,
        mirror: read_inventory(&mut r)?,
        cursor: read_cursor(&mut r)?,
        expected_baseline_version: r.num()?,
        plan_sequence: r.num()?,
        limits: read_limits(&mut r)?,
    };
    r.end()?;
    // 规范顺序与二进制唯一编码；拒绝重复、非规范 VaultPath 以及其他歧义。
    if encode(&value)? != bytes {
        return Err(BridgeError::CorruptStore);
    }
    Ok(value)
}

pub(crate) fn binding(w: &mut Writer, b: &Binding) -> Result<()> {
    w.raw(&b.link_id)?;
    w.raw(&b.canonical_vault)?;
    w.raw(&b.remote_vault)?;
    w.raw(&b.canonical_root_token)?;
    w.raw(&b.mirror_root_token)?;
    w.raw(&b.scope_hash)?;
    w.num(b.scope_epoch)
}
pub(crate) fn read_binding(r: &mut Reader<'_>) -> Result<Binding> {
    Ok(Binding {
        link_id: r.fixed()?,
        canonical_vault: r.fixed()?,
        remote_vault: r.fixed()?,
        canonical_root_token: r.fixed()?,
        mirror_root_token: r.fixed()?,
        scope_hash: r.fixed()?,
        scope_epoch: r.num()?,
    })
}
pub(crate) fn limits(w: &mut Writer, l: Limits) -> Result<()> {
    w.num(l.max_paths as u64)?;
    w.num(l.max_outputs as u64)?;
    w.num(l.max_file_bytes)?;
    w.num(l.deletion.max_count)?;
    w.num(u64::from(l.deletion.max_basis_points))
}
pub(crate) fn read_limits(r: &mut Reader<'_>) -> Result<Limits> {
    Ok(Limits {
        max_paths: r.count(MAX_PATHS)?,
        max_outputs: r.count(MAX_PATHS)?,
        max_file_bytes: r.num()?,
        deletion: DeleteLimits {
            max_count: r.num()?,
            max_basis_points: u16::try_from(r.num()?).map_err(|_| BridgeError::Budget)?,
        },
    })
}
pub(crate) fn cursor(w: &mut Writer, c: &PlanningCursor) -> Result<()> {
    binding(w, &c.binding)?;
    w.num(c.baseline_version)?;
    w.num(c.last_plan_sequence)?;
    w.num(c.last_canonical_observation)?;
    w.num(c.last_mirror_observation)
}
pub(crate) fn read_cursor(r: &mut Reader<'_>) -> Result<PlanningCursor> {
    Ok(PlanningCursor {
        binding: read_binding(r)?,
        baseline_version: r.num()?,
        last_plan_sequence: r.num()?,
        last_canonical_observation: r.num()?,
        last_mirror_observation: r.num()?,
    })
}
pub(crate) fn baseline(w: &mut Writer, b: &Baseline) -> Result<()> {
    binding(w, &b.binding)?;
    w.num(b.version)?;
    w.num(b.canonical_observation)?;
    w.num(b.mirror_observation)?;
    count(w, b.entries.len())?;
    for (p, pair) in &b.entries {
        path(w, p)?;
        presence(w, &pair.canonical)?;
        presence(w, &pair.mirror)?;
    }
    Ok(())
}
pub(crate) fn read_baseline(r: &mut Reader<'_>) -> Result<Baseline> {
    let mut value = Baseline {
        binding: read_binding(r)?,
        version: r.num()?,
        canonical_observation: r.num()?,
        mirror_observation: r.num()?,
        entries: BTreeMap::new(),
    };
    for _ in 0..r.count(MAX_PATHS)? {
        let path = read_path(r)?;
        let pair = Pair {
            canonical: read_presence(r)?,
            mirror: read_presence(r)?,
        };
        if value.entries.insert(path, pair).is_some() {
            return Err(BridgeError::CorruptStore);
        }
    }
    Ok(value)
}
fn inventory(w: &mut Writer, i: &Inventory) -> Result<()> {
    binding(w, &i.binding)?;
    w.byte(match i.side {
        Side::Canonical => 0,
        Side::Mirror => 1,
    })?;
    w.num(i.sequence)?;
    w.byte(u8::from(i.complete))?;
    count(w, i.entries.len())?;
    for (p, value) in &i.entries {
        path(w, p)?;
        presence(w, value)?;
    }
    Ok(())
}
fn read_inventory(r: &mut Reader<'_>) -> Result<Inventory> {
    let binding = read_binding(r)?;
    let side = match r.byte()? {
        0 => Side::Canonical,
        1 => Side::Mirror,
        _ => return Err(BridgeError::CorruptStore),
    };
    let sequence = r.num()?;
    let complete = match r.byte()? {
        0 => false,
        1 => true,
        _ => return Err(BridgeError::CorruptStore),
    };
    let mut entries = BTreeMap::new();
    for _ in 0..r.count(MAX_PATHS)? {
        let path = read_path(r)?;
        if entries.insert(path, read_presence(r)?).is_some() {
            return Err(BridgeError::CorruptStore);
        }
    }
    Ok(Inventory {
        binding,
        side,
        sequence,
        complete,
        entries,
    })
}
fn count(w: &mut Writer, n: usize) -> Result<()> {
    if n > MAX_PATHS {
        return Err(BridgeError::Budget);
    }
    w.num(n as u64)
}
fn path(w: &mut Writer, p: &VaultPath) -> Result<()> {
    w.text(p.as_str(), 1024)
}
fn read_path(r: &mut Reader<'_>) -> Result<VaultPath> {
    VaultPath::parse(r.text(1024)?).map_err(|_| BridgeError::CorruptStore)
}
fn presence(w: &mut Writer, p: &Presence) -> Result<()> {
    match p {
        Presence::Missing => w.byte(0),
        Presence::Present(v) => {
            w.byte(1)?;
            w.raw(&v.revision)?;
            w.raw(&v.sha256)?;
            w.num(v.bytes)
        }
    }
}
fn read_presence(r: &mut Reader<'_>) -> Result<Presence> {
    match r.byte()? {
        0 => Ok(Presence::Missing),
        1 => Ok(Presence::Present(FileVersion {
            revision: r.fixed()?,
            sha256: r.fixed()?,
            bytes: r.num()?,
        })),
        _ => Err(BridgeError::CorruptStore),
    }
}
