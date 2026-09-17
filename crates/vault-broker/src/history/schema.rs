//! 初始化固定 schema；半初始化或未知目录不被自动覆盖。
use super::store::Binding;
use crate::{
    BrokerError, FsVaultBroker,
    writer::{Identity, directory, disk},
};
use cap_std::fs::Dir;
use std::path::Path;
const NAME: &str = "meta-v1";
const PREPARING: &str = "meta-v1.preparing";
pub(super) fn initialize(broker: &FsVaultBroker, history: &Dir) -> Result<(), BrokerError> {
    // 固定准备目录若已存在就拒绝，不覆盖上次失败或未知版本留下的对象。
    history
        .create_dir(PREPARING)
        .map_err(|e| BrokerError::io("prepare history schema", e))?;
    let staged = directory(history, PREPARING)?;
    for name in ["requests", "catalogs", "outbox"] {
        staged
            .create_dir(name)
            .map_err(|e| BrokerError::io("create history schema", e))?;
    }
    let binding = Binding {
        version: 1,
        pair_id: broker.writer.binding.pair_id,
        vault_uuid: broker.vault_uuid,
        root: Identity::directory(&staged)?,
        requests: Identity::directory(&directory(&staged, "requests")?)?,
        catalogs: Identity::directory(&directory(&staged, "catalogs")?)?,
        outbox: Identity::directory(&directory(&staged, "outbox")?)?,
    };
    disk::create(&staged, "binding.json", &binding, || {
        broker.writer.check(&broker.root, &broker.data)
    })?;
    for name in ["requests", "catalogs", "outbox", "."] {
        FsVaultBroker::sync_directory(&staged, Path::new(name))?;
    }
    broker.writer.check(&broker.root, &broker.data)?;
    history
        .rename(PREPARING, history, NAME)
        .map_err(|e| BrokerError::io("publish history schema", e))?;
    FsVaultBroker::sync_directory(history, Path::new("."))?;
    Ok(())
}
