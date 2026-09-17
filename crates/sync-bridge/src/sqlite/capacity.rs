//! 完整验证时重算终态最坏空间；任何动作获准 issue 前已保留回执容量。
use super::{BridgeError, RECEIPT_MAX, STATE_MAX, input_codec, model::*, state_codec};

// 每路径：UTF-8 字节长前缀、最长 VaultPath、两侧 Present 的 tag/revision/hash/bytes。
const BASELINE_ENTRY_MAX: usize = 8 + 1024 + 2 * (1 + 32 + 32 + 8);
// magic/version/binding/limits/cursor/基线头/选项/floor 与完整 retirement ACK。
// 固定字段实际小于 9 KiB；留 64 KiB 并用编码回归验证，不允许静默扩展 schema。
const CONTROL_MAX: usize = 64 * 1024;
pub(super) const ACTION_MAX: usize = 32768;

pub(super) fn reserved_bytes(s: &State) -> Result<usize> {
    let entries = s
        .limits
        .max_paths
        .checked_mul(BASELINE_ENTRY_MAX)
        .ok_or(BridgeError::Overflow)?;
    let mut size = CONTROL_MAX
        .checked_add(entries)
        .ok_or(BridgeError::Overflow)?;
    // 当前控制状态也必须处于公式承诺内；未来基线按 max_paths 预留，不沿用当前短路径。
    let without_base = State {
        version: s.version,
        binding: s.binding.clone(),
        limits: s.limits,
        cursor: s.cursor.clone(),
        baseline: crate::Baseline {
            binding: s.baseline.binding.clone(),
            version: s.baseline.version,
            canonical_observation: s.baseline.canonical_observation,
            mirror_observation: s.baseline.mirror_observation,
            entries: std::collections::BTreeMap::new(),
        },
        pending: None,
        last: None,
        fuse: None,
        floor: s.floor,
        retirement: s.retirement.clone(),
    };
    if state_codec::encode(&without_base)?.len() + 24 > CONTROL_MAX {
        return Err(BridgeError::Budget);
    }
    if let Some(last) = &s.last {
        add(&mut size, plan_bytes(last, "last")?)?;
        for action in &last.actions {
            add(&mut size, state_codec::progress_bytes(action)?.len() + 16)?;
        }
    }
    if let Some(p) = &s.pending {
        add(
            &mut size,
            input_codec::encode(&p.input)?.len() + "pending".len() + 8 + RECEIPT_MAX,
        )?;
        let actions = p
            .actions
            .len()
            .checked_mul(ACTION_MAX + 16)
            .ok_or(BridgeError::Overflow)?;
        add(&mut size, actions)?;
    }
    if let Some(f) = &s.fuse {
        add(
            &mut size,
            input_codec::encode(&f.input)?.len() + "fuse".len() + 8 + RECEIPT_MAX,
        )?;
    }
    Ok(size)
}
pub(super) fn validate(s: &State) -> Result<()> {
    if reserved_bytes(s)? > STATE_MAX {
        return Err(BridgeError::Budget);
    }
    Ok(())
}
fn plan_bytes(p: &Pending, role: &str) -> Result<usize> {
    Ok(input_codec::encode(&p.input)?.len()
        + role.len()
        + 8
        + p.summary_ack.as_ref().map_or(0, String::len))
}
fn add(size: &mut usize, value: usize) -> Result<()> {
    *size = size.checked_add(value).ok_or(BridgeError::Overflow)?;
    Ok(())
}
