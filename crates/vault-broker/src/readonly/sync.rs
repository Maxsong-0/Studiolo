//! 普通只读根共用完整观察；没有 canonical 身份、格式许可或 Mirror 静止证明。
use super::{ReadOnlyVault, private};
use crate::sync_observation::{
    kernel,
    state::{Proofs, Survey},
};
use crate::{BlobIdentity, BrokerError, SyncReadSelection, VaultPath};
use cap_std::fs::{Dir, Metadata};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// 完整原始字节事实；UTF-8 为真仍不代表 Markdown 或附件格式合规。
/// ```compile_fail
/// fn canonical(value: studiolo_vault_broker::LocalReadFile) { let _ = value.revision; }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalReadFile {
    /// 完整原始 SHA-256 和实际字节数；没有路径/Vault 绑定的 Revision。
    pub identity: BlobIdentity,
    /// 跨块并包括 EOF 的全部 UTF-8 检查结果。
    pub utf8_valid: bool,
}
/// 只有完整名字及固定父链 ENOENT 才能产生 Missing。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalReadPresence {
    /// 已完整读取并复核的事实，不含正文。
    Present(LocalReadFile),
    /// 原完整名字与固定父链共同证明缺失。
    Missing,
}
/// 同 reader 实例封闭的元数据调查，不能构造、反序列化或换范围。
/// ```compile_fail
/// let survey: studiolo_vault_broker::LocalReadSurvey = serde_json::from_str("{}").unwrap();
/// ```
pub struct LocalReadSurvey {
    instance: Arc<()>,
    survey: Survey,
}
impl LocalReadSurvey {
    /// 排序文件集合，仅供汇合；不公开隐藏/排除名字或底层句柄。
    pub fn paths(&self) -> &BTreeSet<VaultPath> {
        &self.survey.paths
    }
}
/// 固定 reader 的完整本地事实；不证明宿主绝对路径静止或远端下载完整。
/// ```compile_fail
/// let proof: studiolo_vault_broker::LocalReadObservation = serde_json::from_str("{}").unwrap();
/// ```
pub struct LocalReadObservation {
    instance: Arc<()>,
    entries: BTreeMap<VaultPath, LocalReadPresence>,
    proofs: Proofs,
}
impl LocalReadObservation {
    /// 完整排序结果；调用方仍须批准联合范围并验证真实 owner 的静止窗口。
    pub fn entries(&self) -> &BTreeMap<VaultPath, LocalReadPresence> {
        &self.entries
    }
}

impl ReadOnlyVault {
    /// 元数据阶段零正文：完整验证范围、预算、名字碰撞和所有访问对象。
    /// 同原读取能力固定根 FD；生产 owner 应另外重验其批准的宿主路径。
    pub fn begin_scoped_observation(
        &self,
        selection: &SyncReadSelection,
        required: &[VaultPath],
    ) -> Result<LocalReadSurvey, BrokerError> {
        private::directory(&self.root, self.private)?;
        let survey = kernel::begin(&self.root, self.private, selection, required)?;
        private::directory(&self.root, self.private)?;
        Ok(LocalReadSurvey {
            instance: Arc::clone(&self.instance),
            survey,
        })
    }

    /// 消费调查，仅增补原范围内完整并集；不重枚举、不伪造 Missing 或 canonical revision。
    pub fn finish_scoped_observation(
        &self,
        survey: LocalReadSurvey,
        additional_required: &[VaultPath],
    ) -> Result<LocalReadObservation, BrokerError> {
        self.scoped_gate(&survey.instance)?;
        let prepared =
            kernel::finish(&self.root, self.private, survey.survey, additional_required)?;
        let mut result = LocalReadObservation {
            instance: survey.instance,
            entries: BTreeMap::new(),
            proofs: prepared.proofs,
        };
        for (path, candidate) in prepared.candidates {
            self.scoped_gate(&result.instance)?;
            let (fact, proof) = kernel::read(
                &self.root,
                self.private,
                &path,
                candidate,
                &prepared.selection,
                None,
            )?;
            let presence = match fact {
                Some(fact) => LocalReadPresence::Present(LocalReadFile {
                    identity: fact.identity,
                    utf8_valid: fact.utf8_valid,
                }),
                None => LocalReadPresence::Missing,
            };
            result.entries.insert(path.clone(), presence);
            result.proofs.files.insert(path, proof);
        }
        #[cfg(test)]
        crate::sync_observation::tests::hook(2);
        self.revalidate_scoped_observation(&result)?;
        Ok(result)
    }

    /// 不重读正文：重验同 reader、私有模式、全部目录/文件/缺失父链。
    pub fn revalidate_scoped_observation(
        &self,
        observation: &LocalReadObservation,
    ) -> Result<(), BrokerError> {
        self.scoped_gate(&observation.instance)?;
        kernel::revalidate(&self.root, self.private, &observation.proofs)?;
        self.scoped_gate(&observation.instance)
    }
    fn scoped_gate(&self, instance: &Arc<()>) -> Result<(), BrokerError> {
        if !Arc::ptr_eq(&self.instance, instance) {
            return Err(BrokerError::CaptureChanged);
        }
        private::directory(&self.root, self.private)
    }
}

// 内核仅调用已有私有模式检查；不开放 FD、任意回调或新权限。
pub(crate) fn check_directory(dir: &Dir, enabled: bool) -> Result<(), BrokerError> {
    private::directory(dir, enabled)
}
pub(crate) fn check_file(metadata: &Metadata, enabled: bool) -> Result<(), BrokerError> {
    private::file(metadata, enabled)
}
