//! 持有唯一写者时的完整捕获；观察值不授权，也不冻结管理员带外修改。

use std::sync::Arc;

use crate::{
    BrokerError, CanonicalRead, FsVaultBroker, ReadEntry, ScanLimits, VaultPath, VaultUuid,
    readonly, writer::Identity,
};

#[cfg(test)]
#[path = "../tests/capture_private/mod.rs"]
mod tests;

/// 一次有界扫描的完整结果；正文与条目来自同一次扫描，不含附件字节。
#[derive(Debug)]
pub struct CanonicalCapture {
    /// 已通过安全扫描的可见目录、Markdown 与附件元信息。
    pub entries: Vec<ReadEntry>,
    /// 以真实 Vault UUID、逻辑路径和精确字节计算 revision 的 Markdown。
    pub notes: Vec<CanonicalRead>,
    /// 发布候选前必须交回原 Broker 重验的非授权观测值。
    pub stamp: CaptureStamp,
}

/// 不可构造或反序列化的实例观测；只持内存身份，不延长目录租约。
///
/// 调用者只能从成功捕获获得 stamp，不能自行构造：
/// ```compile_fail
/// use studiolo_vault_broker::CaptureStamp;
/// let stamp = CaptureStamp {};
/// ```
/// 也不能从持久记录或网络输入反序列化出 stamp：
/// ```compile_fail
/// use studiolo_vault_broker::CaptureStamp;
/// let stamp: CaptureStamp = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone)]
pub struct CaptureStamp {
    instance: Arc<()>,
    pair_id: VaultUuid,
    vault_uuid: VaultUuid,
    committed_through: u64,
}

impl std::fmt::Debug for CaptureStamp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CaptureStamp")
            .field("committed_through", &self.committed_through)
            .finish_non_exhaustive()
    }
}

impl CaptureStamp {
    /// 捕获时已确认的最大提交序号，不代表授权或任意外部写入的新鲜度。
    #[must_use]
    pub fn committed_through(&self) -> u64 {
        self.committed_through
    }
}

impl FsVaultBroker {
    /// 在同一写者所有者互斥范围内完整捕获；不清理或补记任何未决操作。
    /// 调用者必须用同一个所有者串行化捕获、最终重验与所有 canonical 写入。
    /// 每项读取有局部稳定性检查，但无法证明带外写入下的全库原子快照。
    pub fn capture(&mut self, limits: ScanLimits) -> Result<CanonicalCapture, BrokerError> {
        let stamp = CaptureStamp {
            instance: Arc::clone(&self.capture_instance),
            pair_id: self.writer.binding.pair_id,
            vault_uuid: self.vault_uuid,
            committed_through: ready(self)?,
        };
        let batch = readonly::scan_bound(&self.root, limits, self.vault_uuid)?;
        #[cfg(test)]
        tests::after_read();
        self.revalidate_capture(&stamp)?;
        if !batch.diagnostics.is_empty() {
            return Err(BrokerError::IncompleteCapture);
        }
        Ok(CanonicalCapture {
            entries: batch.entries,
            notes: batch
                .notes
                .into_iter()
                .map(|note| CanonicalRead {
                    vault_uuid: self.vault_uuid,
                    path: note.path,
                    content: note.content,
                    revision: note.revision,
                })
                .collect(),
            stamp,
        })
    }

    /// 重验实例、永久配对、根身份、未决和水位；不重扫文件，也不是写入授权。
    /// 重开后的实例不能接受旧 stamp，即使配对及水位相同也必须重新捕获。
    pub fn revalidate_capture(&self, stamp: &CaptureStamp) -> Result<(), BrokerError> {
        let committed = ready(self)?;
        if !Arc::ptr_eq(&self.capture_instance, &stamp.instance)
            || self.writer.binding.pair_id != stamp.pair_id
            || self.vault_uuid != stamp.vault_uuid
            || committed != stamp.committed_through
        {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(())
    }

    /// 在原写者内有界读取可见文件；未知安装、绑定变化或已观察倒退均失败关闭。
    /// Prepared 尚未安装，可以读取，但不能借此启动另一笔写入。
    /// 预算直接约束读取缓冲，不先按协议写入上限读满再截断。
    pub fn read_file(
        &self,
        path: &VaultPath,
        max_bytes: u64,
    ) -> Result<CanonicalRead, BrokerError> {
        let committed = ready(self)?;
        let note = readonly::read_bound(&self.root, path, max_bytes, self.vault_uuid)?;
        #[cfg(test)]
        tests::after_read();
        if ready(self)? != committed {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(CanonicalRead {
            vault_uuid: self.vault_uuid,
            path: note.path,
            content: note.content,
            revision: note.revision,
        })
    }

    /// 可信启动核对批准监视根与实际所持 canonical/data 根，不返回路径或句柄。
    pub fn validate_root_identities(
        &self,
        canonical: (u64, u64),
        data: (u64, u64),
    ) -> Result<(), BrokerError> {
        self.writer.check(&self.root, &self.data)?;
        let actual_root = Identity::directory(&self.root)?;
        let actual_data = Identity::directory(&self.data)?;
        if (actual_root.dev, actual_root.ino) != canonical
            || (actual_data.dev, actual_data.ino) != data
        {
            return Err(BrokerError::WriterBindingMismatch);
        }
        self.writer.check(&self.root, &self.data)
    }
}

pub(crate) fn ready(broker: &FsVaultBroker) -> Result<u64, BrokerError> {
    let state = broker.write_status()?;
    if state.read_blocked {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(state.committed_through)
}
