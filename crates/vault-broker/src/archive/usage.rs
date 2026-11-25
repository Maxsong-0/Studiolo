//! 写者记录离各自上限还有多远。只看文件名和大小，不读内容；健康页和「要不要打包」都用它。
use super::format::MAX_SEGMENTS;
use super::loose;
use crate::{
    BrokerError, FsVaultBroker, file_operation::disk::Folder, records::WriterRecords, writer::disk,
};

/// 一处用了多少、上限多少。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct RecordUsageItem {
    /// 现在用了多少。
    pub used: u64,
    /// 到多少就拒绝新写入。
    pub limit: u64,
}

/// 写者记录各处的用量。打了包的不占散件的名额。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct RecordUsage {
    /// 正文描述符条数。
    pub descriptors: RecordUsageItem,
    /// 正文描述符字节（含写入时给两份回执留的空）。
    pub descriptor_bytes: RecordUsageItem,
    /// 幂等重放条数。
    pub replays: RecordUsageItem,
    /// 幂等重放字节。
    pub replay_bytes: RecordUsageItem,
    /// 文件操作请求条数；没启用是 0 比上限。
    pub file_operations: RecordUsageItem,
    /// 文件操作请求字节。
    pub file_operation_bytes: RecordUsageItem,
    /// 墓碑条数，上限取批准的名额；没启用时上限为 0。
    pub tombstones: RecordUsageItem,
    /// 打包的段数。
    pub segments: RecordUsageItem,
    /// 打包的包数。
    pub packs: u64,
}
impl RecordUsage {
    /// 打包能腾出来的三处里，用得最满的那处到了一半。
    #[must_use]
    pub fn half_full(&self) -> bool {
        [
            self.descriptors,
            self.descriptor_bytes,
            self.replays,
            self.replay_bytes,
            self.file_operations,
            self.file_operation_bytes,
        ]
        .iter()
        .any(|item| item.used * 2 >= item.limit)
    }
}

impl FsVaultBroker {
    /// 数一遍散件，加上打包的段数。
    pub fn record_usage(&self) -> Result<RecordUsage, BrokerError> {
        self.writer.check(&self.root, &self.data)?;
        let limits = crate::file_operation::store::open(self)?.map(|roots| roots.limits);
        let archive = self.archive()?;
        let (descriptors, descriptor_bytes) = count(self, "history/meta-v1/requests", |name| {
            name.ends_with(".json")
        })?;
        let (replays, replay_bytes) = count(self, "idempotency", |_| true)?;
        let (files, file_bytes) = count(self, "file-operations-v1/records", |name| {
            !name.starts_with('.')
        })?;
        let (tombstones, _) = count(self, "file-operations-v1/tombstones", |name| {
            !name.starts_with('.')
        })?;
        let (records, record_bytes) = Folder::Records.budget();
        let item = |used, limit: usize| RecordUsageItem {
            used,
            limit: limit as u64,
        };
        Ok(RecordUsage {
            descriptors: item(descriptors, disk::REPLAY_COUNT),
            descriptor_bytes: item(
                descriptor_bytes + descriptors * crate::recorded::capacity::ACK_BYTES as u64,
                crate::recorded::capacity::DESCRIPTOR_BYTES,
            ),
            replays: item(replays, disk::REPLAY_COUNT),
            replay_bytes: item(replay_bytes, disk::REPLAY_BYTES),
            file_operations: item(files, records),
            file_operation_bytes: item(file_bytes, record_bytes),
            tombstones: item(
                tombstones,
                limits.map_or(0, |limits| limits.max_tombstones as usize),
            ),
            segments: item(
                archive.head().map_or(0, |head| u64::from(head.segments)),
                MAX_SEGMENTS as usize,
            ),
            packs: archive.head().map_or(0, |head| head.packs),
        })
    }
}

/// 目录里符合的文件有几个、共几字节。
fn count(
    broker: &FsVaultBroker,
    path: &str,
    wanted: impl Fn(&str) -> bool,
) -> Result<(u64, u64), BrokerError> {
    let Some(dir) = loose::open(&broker.data, path)? else {
        return Ok((0, 0));
    };
    let mut total = (0u64, 0u64);
    for name in loose::names(&broker.data, path)? {
        if wanted(&name) {
            let metadata = dir
                .symlink_metadata(&name)
                .map_err(|e| BrokerError::io("stat writer record", e))?;
            total.0 += 1;
            total.1 += metadata.len();
        }
    }
    Ok(total)
}
