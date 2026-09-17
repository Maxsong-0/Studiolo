//! 历史目录的替换验证原代次及打开对象，避免覆盖另一份已变化的索引。
use super::store::{CATALOG_BYTES, open};
use crate::{
    BrokerError, FsVaultBroker,
    writer::{directory, disk},
};
use serde::Serialize;
use std::path::Path;
/// 目录更新绑定读取时的代次；同内容换 inode 在最终检查后也不能偷偷替代已打开对象。
pub(crate) fn save_catalog<T: Serialize>(
    broker: &FsVaultBroker,
    name: &str,
    value: &T,
    expected: Option<&str>,
) -> Result<(), BrokerError> {
    let bytes = serde_json::to_vec(value).map_err(|_| BrokerError::CorruptWriterState)?;
    if bytes.len() > CATALOG_BYTES {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let dir = open(broker, true)?.ok_or(BrokerError::CorruptWriterState)?;
    let basename = name
        .strip_prefix("catalogs/")
        .ok_or(BrokerError::CorruptWriterState)?;
    if basename.contains('/') {
        return Err(BrokerError::CorruptWriterState);
    }
    let parent = directory(&dir, "catalogs")?;
    let observed = disk::read_identified(&parent, basename, CATALOG_BYTES)?;
    if observed
        .as_ref()
        .map(|value| crate::sha256(&value.0))
        .as_deref()
        != expected
    {
        return Err(BrokerError::HistoryChanged);
    }
    let pin = observed.map(|(bytes, identity)| disk::FilePin::from_read(&bytes, identity));
    FsVaultBroker::atomic_write_dir_checked(
        &parent,
        Path::new(basename),
        &bytes,
        expected.is_none(),
        || {
            open(broker, false)?.ok_or(BrokerError::CorruptWriterState)?;
            if let Some(pin) = &pin {
                pin.check(&parent, basename, CATALOG_BYTES)?;
            } else if disk::read(&parent, basename, CATALOG_BYTES)?.is_some() {
                return Err(BrokerError::HistoryChanged);
            }
            Ok(())
        },
    )?;
    open(broker, false)?;
    Ok(())
}
