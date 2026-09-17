//! 全部保全写入结束后冻结旧/新 L0；最终安装只重验身份，不重新哈希大正文。
use crate::blob::{file, store};
use crate::mutation::record::Operation;
use crate::writer::directory;
use crate::{BrokerError, FsVaultBroker, Revision, VaultPath};

pub(crate) struct HistoryPins {
    path: VaultPath,
    previous: Option<Revision>,
    target: Revision,
    objects: Vec<(VaultPath, file::ContentProof)>,
}

impl HistoryPins {
    /// 只能在准备阶段调用；每个不同 revision 完整核验一次，随后冻结父链与文件元数据。
    pub(crate) fn capture(broker: &FsVaultBroker, op: &Operation) -> Result<Self, BrokerError> {
        let history = directory(&broker.data, "history")?;
        let mut objects: Vec<(VaultPath, file::ContentProof)> = Vec::with_capacity(2);
        for revision in op.previous_revision.into_iter().chain([op.target_revision]) {
            let name = store::logical(&revision.to_string())?;
            if objects.iter().any(|(existing, _)| existing == &name) {
                continue;
            }
            let proof = store::verify(broker, &history, name.as_str(), &op.path, revision)?;
            if revision == op.target_revision
                && (proof.identity.bytes != op.stage.size
                    || proof.identity.sha256 != op.stage.digest)
            {
                return Err(BrokerError::CorruptWriterState);
            }
            objects.push((name, proof));
        }
        let pins = Self {
            path: op.path.clone(),
            previous: op.previous_revision,
            target: op.target_revision,
            objects,
        };
        // 核验第二个对象期间，第一个对象或公共目录也不能发生变化。
        pins.check(broker, op)?;
        Ok(pins)
    }

    /// 只有有限的 open/stat 和父链检查；不得在最终准入之后读取或重算 L0 正文。
    pub(crate) fn check(&self, broker: &FsVaultBroker, op: &Operation) -> Result<(), BrokerError> {
        broker.writer.check(&broker.root, &broker.data)?;
        if self.path != op.path
            || self.previous != op.previous_revision
            || self.target != op.target_revision
        {
            return Err(BrokerError::CaptureChanged);
        }
        let history = directory(&broker.data, "history")?;
        for (name, proof) in &self.objects {
            file::check(&history, name, &proof.node)?;
        }
        broker.writer.check(&broker.root, &broker.data)
    }

    #[cfg(test)]
    pub(crate) fn object_count(&self) -> usize {
        self.objects.len()
    }
}
