//! 由审批与已核验根事实派生 Bridge 绑定和保守预算；没有 IO。

use sha2::{Digest, Sha256};
use studiolo_host_config::{SyncScope, sync_transport::ApprovedTransport};
use studiolo_sync_bridge::{Binding, Limits};

/// Bridge 持久状态的绑定：链身份、Vault 身份与两个根 token 全部由审批与已核验根身份派生。
///
/// `canonical_identity` 是可信启动层（`ServingRoots`）或维护入口（`CanonicalServingRoots`）
/// 核验过的 canonical 根 `(dev, ino)`。组合根与 `studiolo-maintain sync-init` 必须算出同一个
/// 绑定，否则 `BridgeState::open_existing` 按绑定不符拒绝。
#[must_use]
pub fn bridge_binding(approval: &ApprovedTransport<'_>, canonical_identity: (u64, u64)) -> Binding {
    let scope = approval.binding();
    let roots = approval.roots();
    Binding {
        link_id: scope.link_id,
        canonical_vault: vault_identity(&scope.vault_uuid.to_string()),
        remote_vault: scope.remote_vault,
        canonical_root_token: root_token("canonical", canonical_identity),
        mirror_root_token: root_token("mirror", roots.mirror_identity()),
        scope_hash: scope.scope_hash,
        scope_epoch: scope.scope_epoch,
    }
}

/// Bridge 执行器的保守预算：路径与输出上限固定，单对象字节上限取范围内最大的 `max_bytes`。
#[must_use]
pub fn bridge_limits(scope: &SyncScope) -> Limits {
    Limits::conservative(scope.max_bytes())
}

/// 根 token：域分隔 sha256(角色 ‖ dev ‖ ino)。
fn root_token(role: &str, identity: (u64, u64)) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"studiolo.sync-plane.root.v1\0");
    hash.update(role.as_bytes());
    hash.update(identity.0.to_be_bytes());
    hash.update(identity.1.to_be_bytes());
    hash.finalize().into()
}

/// Vault 身份的 16 字节投影：域分隔 sha256 的前 16 字节。
fn vault_identity(uuid: &str) -> [u8; 16] {
    let mut hash = Sha256::new();
    hash.update(b"studiolo.sync-plane.vault.v1\0");
    hash.update(uuid.as_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    let mut identity = [0u8; 16];
    identity.copy_from_slice(&digest[..16]);
    identity
}
