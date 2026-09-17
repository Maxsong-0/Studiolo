//! 显式只读来源固定绝对根和逻辑路径，不暴露 FD、读取回调或可写 stage。
use super::{BlobIdentity, StreamLimits, file};
use crate::{BrokerError, VaultPath, VaultUuid, writer::DirectoryPin};
use cap_std::fs::{Dir, File};
use std::path::Path;

/// 一个只读附件来源；不会创建身份、控制目录或修改来源内容。
pub struct BlobSource {
    root: Dir,
    pin: DirectoryPin,
    path: VaultPath,
    node: file::NodeProof,
}
impl BlobSource {
    /// 显式绝对根逐级拒绝链接；路径限于该根内的可见普通文件。
    pub fn open(root: &Path, path: VaultPath) -> Result<Self, BrokerError> {
        file::path_budget(&path)?;
        if path.as_str().split('/').any(|part| part.starts_with('.')) {
            return Err(BrokerError::ProtectedPath(path));
        }
        let (root, pin) = crate::writer::open_blob_source_root(root)?;
        let node = file::observe_node(&root, &path)?;
        pin.check()?;
        Ok(Self {
            root,
            pin,
            path,
            node,
        })
    }
    /// 完整分块计算原始字节身份；返回前重验来源路径、根与稳定性。
    pub fn identity(&self, limits: StreamLimits) -> Result<BlobIdentity, BrokerError> {
        limits.validate()?;
        self.pin.check()?;
        file::check(&self.root, &self.path, &self.node)?;
        let proof = file::scan(&self.root, &self.path, limits.effective_limit, None)?;
        self.pin.check()?;
        if proof.node != self.node {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(proof.identity)
    }
    pub(crate) fn transfer(
        &self,
        output: &mut File,
        limits: StreamLimits,
        vault: VaultUuid,
        target: &VaultPath,
        expected: &BlobIdentity,
    ) -> Result<file::ContentProof, BrokerError> {
        limits.validate()?;
        expected.validate()?;
        self.pin.check()?;
        file::check(&self.root, &self.path, &self.node)?;
        if expected.bytes > limits.effective_limit {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let proof = file::transfer(
            &self.root,
            &self.path,
            Some(output),
            expected.bytes,
            Some((vault, target)),
        )?;
        self.pin.check()?;
        if proof.node != self.node {
            return Err(BrokerError::CaptureChanged);
        }
        if &proof.identity != expected {
            return Err(BrokerError::IdempotencyConflict);
        }
        Ok(proof)
    }
    pub(crate) fn check(&self, proof: &file::ContentProof) -> Result<(), BrokerError> {
        self.pin.check()?;
        file::check(&self.root, &self.path, &proof.node)?;
        self.pin.check()
    }
}
