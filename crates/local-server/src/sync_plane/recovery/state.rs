//! `plane-state.json` 的纯编解码：版本、校验和、集合的规范化与形态自检；没有 IO。
//!
//! 撕裂、未知字段、版本不符、集合乱序、冲突戳畸形或校验和不符一律失败关闭，绝不把半写洗成「首次运行」。
//!
//! **追加字段兼容规则**：版本仍是 1。版本 1 之后追加的字段（`headless_excluded`、`conflict_stamps`）
//! 缺省为空、为空时不写出，并且只在非默认时各带一个标签字节追加进校验和：旧文件（没有这些字段）
//! 解码后算出的校验和与旧实现逐字节相同，照常解码；字段全空时写出的文件，旧实现也读得回。
//! 以后再追加字段照此办理：只在非默认时进校验和，标签字节不复用。

use super::{MAX_CONFLICT_STAMPS, PlaneState};
use crate::sync_plane::PlaneError;
use sha2::{Digest, Sha256};
use studiolo_sync_transport::owned_run::SyncMode;

pub(super) const VERSION: u8 = 1;
const DOMAIN: &[u8] = b"studiolo.sync-plane.state.v1\0";
/// 追加字段 `headless_excluded` 在校验和里的标签字节。
const TAG_HEADLESS_EXCLUDED: u8 = 1;
/// 追加字段 `conflict_stamps` 在校验和里的标签字节。
const TAG_CONFLICT_STAMPS: u8 = 2;

/// 补齐版本与校验和，并把两个排除集合排序去重；每次持久化前都过这里。
pub(super) fn seal(mut state: PlaneState) -> PlaneState {
    state.version = VERSION;
    state.verified_excluded.sort();
    state.verified_excluded.dedup();
    state.headless_excluded.sort();
    state.headless_excluded.dedup();
    state.checksum = checksum(&state);
    state
}

/// 解码持久字节；任何不完整或不认识的内容都失败关闭。
pub(super) fn decode(bytes: &[u8]) -> Result<PlaneState, PlaneError> {
    let state: PlaneState = serde_json::from_slice(bytes).map_err(|_| PlaneError::Storage)?;
    if state.version != VERSION || state.checksum != checksum(&state) {
        return Err(PlaneError::Storage);
    }
    validate(&state)?;
    Ok(state)
}

/// 规范形态自检（解码与写盘共用）：两个排除集合严格升序；冲突戳不超过条目上限，
/// 键是 64 位小写十六进制、值是 12 位数字。
pub(super) fn validate(state: &PlaneState) -> Result<(), PlaneError> {
    let ascending = |set: &[String]| set.windows(2).all(|pair| pair[0] < pair[1]);
    let stamps = state.conflict_stamps.len() <= MAX_CONFLICT_STAMPS
        && state.conflict_stamps.iter().all(|(key, stamp)| {
            key.len() == 64
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                && stamp.len() == 12
                && stamp.bytes().all(|byte| byte.is_ascii_digit())
        });
    if ascending(&state.verified_excluded) && ascending(&state.headless_excluded) && stamps {
        Ok(())
    } else {
        Err(PlaneError::Storage)
    }
}

fn checksum(state: &PlaneState) -> String {
    let mut digest = Sha256::new();
    digest.update(DOMAIN);
    digest.update([VERSION, u8::from(state.last_run_clean)]);
    digest.update([u8::from(state.started_sequence.is_some())]);
    digest.update(state.started_sequence.unwrap_or_default().to_be_bytes());
    digest.update([u8::from(state.last_exit_unix.is_some())]);
    digest.update(state.last_exit_unix.unwrap_or_default().to_be_bytes());
    digest.update(state.consecutive_rc1.to_be_bytes());
    digest.update([u8::from(state.not_before_unix.is_some())]);
    digest.update(state.not_before_unix.unwrap_or_default().to_be_bytes());
    folders(&mut digest, &state.verified_excluded);
    digest.update([match state.applied_mode {
        None => 0,
        Some(SyncMode::MirrorRemote) => 1,
        Some(SyncMode::Bidirectional) => 2,
    }]);
    // 追加字段：只在非默认时进校验和，旧文件的校验和因此不变。
    if !state.headless_excluded.is_empty() {
        digest.update([TAG_HEADLESS_EXCLUDED]);
        folders(&mut digest, &state.headless_excluded);
    }
    if !state.conflict_stamps.is_empty() {
        digest.update([TAG_CONFLICT_STAMPS]);
        digest.update((state.conflict_stamps.len() as u64).to_be_bytes());
        for (key, stamp) in &state.conflict_stamps {
            digest.update((key.len() as u64).to_be_bytes());
            digest.update(key.as_bytes());
            digest.update(stamp.as_bytes());
        }
    }
    super::stage::hex(digest.finalize().into())
}

/// 一个排除集合：条数，再逐项「长度＋字节」。
fn folders(digest: &mut Sha256, set: &[String]) {
    digest.update((set.len() as u64).to_be_bytes());
    for folder in set {
        digest.update((folder.len() as u64).to_be_bytes());
        digest.update(folder.as_bytes());
    }
}
