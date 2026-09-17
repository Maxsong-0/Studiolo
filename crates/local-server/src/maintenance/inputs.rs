//! 维护只打开经过批准的既有根；不创建存储，也不读取种子或装配应用会话。

use super::{MaintenanceError, options::Options};
use studiolo_audit_ledger::{AuditLedger, SqliteAuditLedger};
use studiolo_host_config::{CanonicalServingRoots, Config};
use studiolo_vault_broker::FsVaultBroker;

pub(super) struct Inputs {
    pub(super) roots: CanonicalServingRoots,
    pub(super) broker: FsVaultBroker,
    pub(super) audit: Option<Box<dyn AuditLedger>>,
}

impl Inputs {
    pub(super) fn open(options: &Options) -> Result<Self, MaintenanceError> {
        let config = Config::load(&options.config).map_err(|_| MaintenanceError::Configuration)?;
        let roots = config
            .canonical_serving_roots(&options.vault, &options.assets, &options.writer_data)
            .map_err(|_| MaintenanceError::Configuration)?;
        roots
            .revalidate()
            .map_err(|_| MaintenanceError::Configuration)?;
        // 普通服务持有相同 Broker 租约时在这里失败；维护命令不会替用户停止服务。
        let broker =
            FsVaultBroker::open_existing(roots.vault_directory(), roots.writer_data_directory())
                .map_err(|_| MaintenanceError::Broker)?;
        broker
            .validate_root_identities(roots.vault_identity(), roots.writer_data_identity())
            .map_err(|_| MaintenanceError::Broker)?;
        roots
            .revalidate()
            .map_err(|_| MaintenanceError::Configuration)?;
        let audit = SqliteAuditLedger::open_existing(config.audit_directory())
            .ok()
            .map(|ledger| Box::new(ledger) as Box<dyn AuditLedger>);
        let inputs = Self {
            roots,
            broker,
            audit,
        };
        inputs.revalidate()?;
        Ok(inputs)
    }

    pub(super) fn revalidate(&self) -> Result<(), MaintenanceError> {
        self.roots
            .revalidate()
            .map_err(|_| MaintenanceError::Configuration)?;
        self.broker
            .validate_root_identities(
                self.roots.vault_identity(),
                self.roots.writer_data_identity(),
            )
            .map_err(|_| MaintenanceError::Broker)?;
        self.roots
            .revalidate()
            .map_err(|_| MaintenanceError::Configuration)
    }
}
