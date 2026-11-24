//! 「本进程是唯一一任」的凭据：只能从还握着主库锁的工作台借出来。

use super::CanonicalWorkbench;
use crate::LocalError;
use std::sync::MutexGuard;
use studiolo_vault_broker::FsVaultBroker;

/// 借着它的期间 Broker 不会被收走，主库 Vault 根目录的内核锁就放不开；另一任起不来。
/// 唯一的构造处是 [`CanonicalWorkbench::sole_instance`]。
pub(crate) struct SoleInstance<'a> {
    _owner: MutexGuard<'a, Option<FsVaultBroker>>,
}

impl CanonicalWorkbench {
    /// 写者已经在关、Broker 已经交出（封存的冻结窗口里）、或者批准的根变了，都借不到。
    pub(crate) fn sole_instance(&self) -> Result<SoleInstance<'_>, LocalError> {
        let owner = self
            .core
            .owner
            .lock()
            .map_err(|_| LocalError::Unavailable)?;
        let broker = owner.as_ref().ok_or(LocalError::Unavailable)?;
        if self
            .core
            .control
            .lock()
            .map_err(|_| LocalError::Unavailable)?
            .closing
        {
            return Err(LocalError::Unavailable);
        }
        self.core
            .roots
            .revalidate()
            .map_err(|_| LocalError::Configuration)?;
        broker.validate_root_identities(
            self.core.roots.vault_identity(),
            self.core.roots.writer_data_identity(),
        )?;
        Ok(SoleInstance { _owner: owner })
    }
}
