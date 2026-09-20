//! 元数据按不可变 manifest-last 安装，同目录 fsync，不经过正文/L0 写入。
use super::{GatewayCallReceipt, GatewayCallSeal, GatewayPublisher};
use crate::{
    BrokerError, FsVaultBroker, MemoryVaultBroker, VaultPath, VaultUuid,
    writer::{Identity, directory, disk},
};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format_version: u32,
    vault_uuid: VaultUuid,
    call: GatewayCallSeal,
}
fn text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
fn encode(
    uuid: VaultUuid,
    seal: GatewayCallSeal,
) -> Result<(GatewayCallReceipt, Vec<u8>), BrokerError> {
    if !text(&seal.call_id, 512)
        || !text(&seal.audit_intent, 512)
        || !text(&seal.model_provider, 128)
        || !text(&seal.model, 256)
        || !disk::hex(&seal.raw_sha256)
        || !disk::hex(&seal.source_manifest_sha256)
        || !disk::hex(&seal.snapshot_id)
        || !disk::hex(&seal.effective_policy_hash)
        || seal.retain_until_unix_seconds <= 0
    {
        return Err(BrokerError::InvalidArtifact(
            "Gateway 封存元数据无效".into(),
        ));
    }
    let manifest_path = VaultPath::parse(format!(
        ".workbench/gateway/{}/{}/{}/manifest.json",
        seal.task_id,
        seal.attempt_id,
        crate::sha256(seal.call_id.as_bytes())
    ))
    .map_err(|_| BrokerError::InvalidArtifact("Gateway 派生路径无效".into()))?;
    let bytes = serde_json::to_vec(&Manifest {
        format_version: 1,
        vault_uuid: uuid,
        call: seal,
    })
    .map_err(|_| BrokerError::CorruptWriterState)?;
    if bytes.len() > disk::RECORD_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok((
        GatewayCallReceipt {
            manifest_path,
            sha256: crate::sha256(&bytes),
        },
        bytes,
    ))
}
fn same(existing: &[u8], expected: &[u8]) -> Result<(), BrokerError> {
    if existing != expected {
        return Err(BrokerError::IdempotencyConflict);
    }
    Ok(())
}
fn ready(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    if broker.write_status()?.unresolved {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(())
}
fn parents(
    broker: &FsVaultBroker,
    path: &VaultPath,
) -> Result<Vec<(String, Identity)>, BrokerError> {
    Ok(crate::blob::file::parents(&broker.root, path)?
        .into_iter()
        .map(|(path, stamp)| (path, stamp.identity))
        .collect())
}
fn check_parents(broker: &FsVaultBroker, pins: &[(String, Identity)]) -> Result<(), BrokerError> {
    ready(broker)?;
    for (path, expected) in pins {
        let actual = directory(&broker.root, if path.is_empty() { "." } else { path })?;
        if Identity::directory(&actual)? != *expected {
            return Err(BrokerError::WriterBindingMismatch);
        }
    }
    Ok(())
}
impl GatewayPublisher for FsVaultBroker {
    fn seal_gateway_call(
        &mut self,
        seal: GatewayCallSeal,
    ) -> Result<GatewayCallReceipt, BrokerError> {
        ready(self)?;
        let (receipt, bytes) = encode(self.vault_uuid, seal)?;
        self.validate_components(&receipt.manifest_path, true)?;
        self.ensure_parent_dirs(&receipt.manifest_path)?;
        let pins = parents(self, &receipt.manifest_path)?;
        let (parent, name) = receipt
            .manifest_path
            .as_str()
            .rsplit_once('/')
            .ok_or(BrokerError::CorruptWriterState)?;
        let dir = directory(&self.root, parent)?;
        match disk::read(&dir, name, disk::RECORD_LIMIT)? {
            Some(existing) => same(&existing, &bytes)?,
            None => {
                check_parents(self, &pins)?;
                // 唯一 commit point：完整元数据同步以后原子安装 manifest；没有 raw 候选。
                Self::atomic_write_dir_checked(&dir, Path::new(name), &bytes, true, || {
                    check_parents(self, &pins)
                })?;
            }
        }
        check_parents(self, &pins)?;
        let stored =
            disk::read(&dir, name, disk::RECORD_LIMIT)?.ok_or(BrokerError::CorruptWriterState)?;
        same(&stored, &bytes)?;
        // 重放也明确同步，覆盖此前安装成功但 fsync 回执中断的窗口。
        let logical = VaultPath::parse(name).map_err(|_| BrokerError::CorruptWriterState)?;
        Self::open_rooted_file(&dir, &logical)?
            .sync_all()
            .map_err(|e| BrokerError::io("sync gateway manifest", e))?;
        Self::sync_directory(&dir, Path::new("."))?;
        check_parents(self, &pins)?;
        Ok(receipt)
    }
}
impl GatewayPublisher for MemoryVaultBroker {
    fn seal_gateway_call(
        &mut self,
        seal: GatewayCallSeal,
    ) -> Result<GatewayCallReceipt, BrokerError> {
        let (receipt, bytes) = encode(self.vault_uuid, seal)?;
        if let Some(existing) = self.gateway_seals.get(&receipt.manifest_path) {
            same(existing, &bytes)?;
        } else {
            self.gateway_seals
                .insert(receipt.manifest_path.clone(), bytes);
        }
        Ok(receipt)
    }
}
