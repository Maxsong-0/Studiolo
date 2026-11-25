//! 写者持久记录的只读来源：正在服务的写者，或封存时从定格导出的副本。
//!
//! 逐条核回执、重放、文件操作和维护槽的读取函数只接它：同一套代码既核活的写者、也核副本，强度不分两份。
//! 这里没有写入能力；写入函数仍只接 [`FsVaultBroker`]。
use crate::{
    BrokerError, FsVaultBroker, VaultUuid, archive::Archive, file_operation, history, maintenance,
    writer::Identity,
};
use cap_std::fs::Dir;
use std::sync::{Arc, Mutex};

pub(crate) trait WriterRecords {
    /// 笔记库根。
    fn root(&self) -> &Dir;
    /// 写者数据根。
    fn data(&self) -> &Dir;
    fn vault_uuid(&self) -> VaultUuid;
    fn pair_id(&self) -> VaultUuid;
    /// 这份来源还是打开时的那一份。
    fn check(&self) -> Result<(), BrokerError>;
    /// 绑定文件里记着的目录身份该和谁比：写者是目录现在的身份，副本是定格那一刻原目录的身份。
    fn bound_identity(&self, dir: &Dir) -> Result<Identity, BrokerError>;
    fn history_pin(&self) -> &Mutex<Option<history::store::Pin>>;
    fn maintenance_pin(&self) -> &Mutex<Option<maintenance::store::Pin>>;
    fn file_operation_pin(&self) -> &Mutex<Option<file_operation::binding::Pin>>;
    /// 打包的旧记录；没打过包就是空的。每次读记录都是散件优先、没有再查它。
    fn archive(&self) -> Result<Arc<Archive>, BrokerError>;
    /// 有一笔写入不知道落没落成。
    fn uncertain(&self) -> bool;
    /// 每次读到的已完成水位都交给来源：写者拒绝倒退，副本只认定格那一刻的值。
    fn observe_committed(&self, committed: u64) -> Result<(), BrokerError>;
}

impl WriterRecords for FsVaultBroker {
    fn root(&self) -> &Dir {
        &self.root
    }
    fn data(&self) -> &Dir {
        &self.data
    }
    fn vault_uuid(&self) -> VaultUuid {
        self.vault_uuid
    }
    fn pair_id(&self) -> VaultUuid {
        self.writer.binding.pair_id
    }
    fn check(&self) -> Result<(), BrokerError> {
        self.writer.check(&self.root, &self.data)
    }
    fn bound_identity(&self, dir: &Dir) -> Result<Identity, BrokerError> {
        Identity::directory(dir)
    }
    fn history_pin(&self) -> &Mutex<Option<history::store::Pin>> {
        &self.history_pin
    }
    fn maintenance_pin(&self) -> &Mutex<Option<maintenance::store::Pin>> {
        &self.maintenance_pin
    }
    fn file_operation_pin(&self) -> &Mutex<Option<file_operation::binding::Pin>> {
        &self.file_operation_pin
    }
    fn archive(&self) -> Result<Arc<Archive>, BrokerError> {
        self.archive
            .lock()
            .map(|archive| Arc::clone(&archive))
            .map_err(|_| BrokerError::CorruptWriterState)
    }
    fn uncertain(&self) -> bool {
        self.writer.uncertain()
    }
    fn observe_committed(&self, committed: u64) -> Result<(), BrokerError> {
        self.writer.observe_committed(committed)
    }
}
