//! 原身份登记只包含逻辑引用、inode 和期限；缺失或损坏不能推定拥有某个目录。
//! 登记目录自己删到一半时，靠改名时写进名字的原 inode 认出来（见 `removing_name`）。
use super::{
    ReaderIdentity, Result, SealedCheckpoint, SealedReference, StagingError, digest,
    io::{Directory, JournalFile},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::time::{SystemTime, UNIX_EPOCH};
use studiolo_core::checkpoint::{CheckpointId, InstantMillis};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    schema: u32,
    checkpoint_id: CheckpointId,
    parent: (u64, u64),
    marker: (u64, u64),
    created_at: InstantMillis,
    reader: Option<(u32, u32)>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RootIdentity {
    pub directory: (u64, u64),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Retained {
    pub reference: SealedReference,
    seal: String,
    registered_at: InstantMillis,
}
impl Identity {
    pub fn new(
        parent: &Directory,
        marker: &Directory,
        checkpoint_id: CheckpointId,
        reader: Option<ReaderIdentity>,
    ) -> Result<Self> {
        Ok(Self {
            schema: 1,
            checkpoint_id,
            parent: parent.identity(),
            marker: marker.identity(),
            created_at: now()?,
            reader: reader.map(|r| (r.producer_uid, r.reader_gid)),
        })
    }
    pub fn key(&self) -> String {
        digest(self.checkpoint_id.as_str().as_bytes())
    }
    pub fn validate(
        &self,
        parent: &Directory,
        marker: &Directory,
        key: &str,
        reader: Option<ReaderIdentity>,
        now: InstantMillis,
    ) -> Result<()> {
        if self.schema != 1
            || self.parent != parent.identity()
            || self.marker != marker.identity()
            || self.key() != key
            || self.reader != reader.map(|r| (r.producer_uid, r.reader_gid))
            || self.created_at.millis() < 0
            || now < self.created_at
        {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
    pub fn retained(&self, sealed: &SealedCheckpoint) -> Result<Retained> {
        let record = Retained {
            reference: sealed.reference().clone(),
            seal: sealed.retention_evidence()?,
            registered_at: now()?,
        };
        record.validate(self, record.registered_at)?;
        Ok(record)
    }
}
impl Retained {
    pub fn expires_at(&self) -> Result<InstantMillis> {
        super::seal::retention_expiry(&self.reference, &self.seal)
    }
    pub fn validate(&self, identity: &Identity, now: InstantMillis) -> Result<()> {
        let hash = &self.reference.seal_sha256;
        if self.reference.checkpoint_id != identity.checkpoint_id
            || !studiolo_core::is_sha256_hex(hash)
            || self.expires_at()?.millis() < 0
            || self.registered_at < identity.created_at
            || now < self.registered_at
        {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
}
/// 删登记目录之前先原子改成的名字的前缀。
pub(super) const REMOVING: &str = "removing-";
/// 删登记目录之前先把它原子改成这个名字，名字里带上它自己的 inode：改名之后不管删到哪一步停下，
/// 回收都只认名字和 inode 对得上的这一个目录，删到只剩空目录也一样。
pub(super) fn removing_name(key: &str, marker: (u64, u64)) -> String {
    format!("{REMOVING}{key}-{}", marker.1)
}
/// `removing_name` 的反解；只认它写出来的那种规范写法（十进制、没有前导零或正号）。
pub(super) fn parse_removing(name: &str) -> Option<(&str, u64)> {
    let (key, inode) = name.strip_prefix(REMOVING)?.split_once('-')?;
    let inode = inode.parse().ok()?;
    (removing_name(key, (0, inode)) == name).then_some((key, inode))
}
pub(super) fn save(directory: &Directory, file: JournalFile, value: &impl Serialize) -> Result<()> {
    let mut bytes = Limited(Vec::new());
    serde_json::to_writer(&mut bytes, value).map_err(|_| StagingError::Limit)?;
    directory.save_journal(file, &bytes.0)
}
pub(super) fn load<T: DeserializeOwned>(
    directory: &Directory,
    file: JournalFile,
) -> Result<Option<T>> {
    directory
        .read_journal(file)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| StagingError::Changed))
        .transpose()
}
pub(super) fn now() -> Result<InstantMillis> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StagingError::Expired)?
        .as_millis();
    Ok(InstantMillis::from_millis(
        i64::try_from(value).map_err(|_| StagingError::Expired)?,
    ))
}
struct Limited(Vec<u8>);
impl std::io::Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > super::io::JOURNAL_LIMIT {
            return Err(std::io::Error::other("生命周期记录超过预算"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
