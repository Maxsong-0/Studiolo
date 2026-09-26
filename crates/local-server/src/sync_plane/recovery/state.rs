//! `plane-state.json` 的纯编解码：版本、校验和、排除项集合的规范化；没有 IO。
//!
//! 撕裂、未知字段、版本不符、集合乱序或校验和不符一律失败关闭，绝不把半写洗成「首次运行」。

use super::PlaneState;
use crate::sync_plane::PlaneError;
use sha2::{Digest, Sha256};
use studiolo_sync_transport::owned_run::SyncMode;

pub(super) const VERSION: u8 = 1;
const DOMAIN: &[u8] = b"studiolo.sync-plane.state.v1\0";

/// 补齐版本与校验和，并把排除项集合排序去重；每次持久化前都过这里。
pub(super) fn seal(mut state: PlaneState) -> PlaneState {
    state.version = VERSION;
    state.verified_excluded.sort();
    state.verified_excluded.dedup();
    state.checksum = checksum(&state);
    state
}

/// 解码持久字节；任何不完整或不认识的内容都失败关闭。
pub(super) fn decode(bytes: &[u8]) -> Result<PlaneState, PlaneError> {
    let state: PlaneState = serde_json::from_slice(bytes).map_err(|_| PlaneError::Storage)?;
    let sorted = state
        .verified_excluded
        .windows(2)
        .all(|pair| pair[0] < pair[1]);
    if state.version != VERSION || !sorted || state.checksum != checksum(&state) {
        return Err(PlaneError::Storage);
    }
    Ok(state)
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
    digest.update((state.verified_excluded.len() as u64).to_be_bytes());
    for folder in &state.verified_excluded {
        digest.update((folder.len() as u64).to_be_bytes());
        digest.update(folder.as_bytes());
    }
    digest.update([match state.applied_mode {
        None => 0,
        Some(SyncMode::MirrorRemote) => 1,
        Some(SyncMode::Bidirectional) => 2,
    }]);
    super::stage::hex(digest.finalize().into())
}
