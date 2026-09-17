//! 严格关联原动作；元数据验证不是对 Broker/mirror 真实来源的认证。

use super::{BridgeError, RECEIPT_MAX, codec::json, input_codec, model::*};
use crate::{Action, ActionKey, Change, Digest, Plan, Side};
use sha2::{Digest as _, Sha256};

pub(crate) fn hash(bytes: &[u8]) -> Digest {
    Sha256::digest(bytes).into()
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn input_hash(input: &crate::Input) -> Result<Digest> {
    Ok(hash(&input_codec::encode(input)?))
}
pub(crate) fn request_hash(p: &Pending, index: usize) -> Result<Digest> {
    let mut bytes = Vec::from(p.cached_hash);
    bytes.extend_from_slice(&(index as u64).to_be_bytes());
    Ok(hash(&bytes))
}
pub(crate) fn external_from_hash(input: &crate::Input, digest: Digest, index: usize) -> String {
    let mut bytes = Vec::from(digest);
    bytes.extend_from_slice(&(index as u64).to_be_bytes());
    format!(
        "bridge-v1-{}-{}-{}",
        hex(&input.scope.binding.link_id),
        input.plan_sequence,
        hex(&hash(&bytes))
    )
}
pub(crate) fn plan(p: &Pending) -> Result<&Plan> {
    Ok(&p.cached_plan)
}
pub(crate) fn index(p: &Pending, key: &ActionKey) -> Result<usize> {
    let i = *p.index.get(&key.path).ok_or(BridgeError::NotFound)?;
    if &p.cached_plan.actions()[i].key != key {
        return Err(BridgeError::NotFound);
    }
    Ok(i)
}
pub(crate) fn validate(p: &Pending, index: usize, receipt: &EffectReceipt) -> Result<()> {
    json(receipt, RECEIPT_MAX)?;
    let plan = plan(p)?;
    let action = plan
        .actions()
        .get(index)
        .ok_or(BridgeError::OutcomeMismatch)?;
    let binding = &p.input.scope.binding;
    let backend = match action.key.target {
        Side::Canonical => Backend::Broker {
            canonical_root: binding.canonical_root_token,
        },
        Side::Mirror => Backend::Mirror {
            mirror_root: binding.mirror_root_token,
        },
    };
    if receipt.external_key != p.actions[index].key
        || receipt.request_digest != request_hash(p, index)?
        || receipt.backend != backend
    {
        return Err(BridgeError::OutcomeMismatch);
    }
    let previous = expected(action);
    match &receipt.outcome {
        EffectOutcome::Committed {
            path,
            previous_revision,
            new_revision,
            content_hash,
            bytes,
            receipt_id,
        } => {
            if path != action.key.path.as_str()
                || previous_revision != &previous
                || !identifier(receipt_id)
            {
                return Err(BridgeError::OutcomeMismatch);
            }
            match &action.change {
                Change::Delete { .. }
                    if new_revision.is_none() && content_hash.is_none() && bytes.is_none() => {}
                Change::CreateOnly { source } | Change::Replace { source, .. }
                    if new_revision.is_some()
                        && *content_hash == Some(source.sha256)
                        && *bytes == Some(source.bytes) => {}
                _ => return Err(BridgeError::OutcomeMismatch),
            }
        }
        EffectOutcome::NoEffect {
            path,
            expected_revision,
            terminal_id,
            ..
        } => {
            if path != action.key.path.as_str()
                || *expected_revision != previous
                || !identifier(terminal_id)
            {
                return Err(BridgeError::OutcomeMismatch);
            }
        }
    }
    Ok(())
}
pub(crate) fn expected(action: &Action) -> Option<Digest> {
    match &action.change {
        Change::CreateOnly { .. } => None,
        Change::Replace {
            target_expected, ..
        }
        | Change::Delete { target_expected } => Some(target_expected.revision),
    }
}
fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
pub(crate) fn known(p: &Pending) -> bool {
    p.unknown_count == 0
}
pub(crate) fn acknowledged(p: &Pending) -> bool {
    known(p) && p.outcome_waiting.is_empty() && p.summary_ack.is_some()
}
pub(crate) fn succeeded(p: &Pending) -> bool {
    known(p) && p.failed_count == 0
}
