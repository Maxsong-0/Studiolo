//! 把 host-config 的封闭批准接到本 crate 的 `ApprovedTransport` 上。
//!
//! 这一层只做投影与摘要；不新增任何根、进程或环境能力。

use super::approval::{
    ApprovedTransport, BackendDescriptor, RootPins, RunBudget, RuntimePins, TransportBindingFacts,
};
use super::error::{Result, TransportError};
use sha2::{Digest, Sha256};
use std::num::NonZeroU64;
use studiolo_host_config::sync_transport::{ApprovedTransport as HostApproved, TransportBackend};

impl ApprovedTransport for HostApproved<'_> {
    fn revalidate(&self) -> Result<()> {
        HostApproved::revalidate(self).map_err(|_| TransportError::ConfigurationChanged)
    }

    fn binding(&self) -> Result<TransportBindingFacts> {
        let binding = HostApproved::binding(self);
        Ok(TransportBindingFacts {
            vault_uuid: binding.vault_uuid,
            link_id: binding.link_id,
            remote_vault: binding.remote_vault,
            // 零代次不能区分「从未设置」与「已回退」，一律拒绝。
            scope_epoch: NonZeroU64::new(binding.scope_epoch)
                .ok_or(TransportError::BindingMismatch)?,
            scope_hash: binding.scope_hash,
            approved_config_digest: self.approval_digest(),
        })
    }

    fn roots(&self) -> RootPins {
        let roots = HostApproved::roots(self);
        RootPins {
            private_root: roots.private_root().to_path_buf(),
            private_identity: roots.private_identity(),
            mirror_root: roots.mirror_root().to_path_buf(),
            mirror_identity: roots.mirror_identity(),
            mirror_view_root: roots.mirror_view_root().to_path_buf(),
        }
    }

    fn runtime(&self) -> RuntimePins {
        let runtime = HostApproved::runtime(self);
        RuntimePins {
            version: runtime.version().to_owned(),
            node_path: runtime.node_path().to_path_buf(),
            cli_path: runtime.cli_path().to_path_buf(),
            closure_digest: runtime.closure_digest(),
        }
    }

    fn backend(&self) -> BackendDescriptor {
        let backend = HostApproved::backend(self);
        BackendDescriptor {
            service_unit: backend.service_unit().to_owned(),
            payload_uid: backend.payload_uid(),
            payload_gid: backend.payload_gid(),
            payload_cgroup_prefix: backend
                .payload_cgroup_prefix()
                .to_string_lossy()
                .into_owned(),
            digest: backend_digest(backend),
        }
    }

    fn budget(&self) -> RunBudget {
        let budget = HostApproved::budget(self);
        RunBudget {
            deadline_seconds: budget.deadline_seconds,
            stop_grace_seconds: budget.stop_grace_seconds,
            stdout_bytes: budget.stdout_bytes,
            stderr_bytes: budget.stderr_bytes,
            config_json_bytes: budget.config_json_bytes,
            private_state_bytes: budget.private_state_bytes,
            log_bytes: budget.log_bytes,
            memory_bytes: budget.memory_bytes,
            pids_max: budget.pids_max,
            cpu_max_percent: budget.cpu_max_percent,
        }
    }
}

/// 后端配置摘要；unit 名、launcher、子组前缀与专用身份任一变化都会改变它。
#[must_use]
pub fn backend_digest(backend: &TransportBackend) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"studiolo.sync-transport.backend.v1\0");
    for field in [
        backend.service_unit().as_bytes(),
        backend.launcher_path().as_os_str().as_encoded_bytes(),
        backend
            .payload_cgroup_prefix()
            .as_os_str()
            .as_encoded_bytes(),
    ] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field);
    }
    hash.update(backend.payload_uid().to_be_bytes());
    hash.update(backend.payload_gid().to_be_bytes());
    hash.finalize().into()
}
