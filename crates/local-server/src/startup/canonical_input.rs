//! 显式既有写者的启动装配；此处不初始化目录、迁移身份或提供失败回退。

use super::StartupError;
use std::sync::Arc;
use studiolo_host_config::CanonicalServingRoots;
use studiolo_local_server::CanonicalWorkbench;
use studiolo_vault_broker::FsVaultBroker;

pub(super) struct Inputs {
    broker: FsVaultBroker,
    roots: Arc<CanonicalServingRoots>,
}

impl Inputs {
    pub(super) fn open(roots: Arc<CanonicalServingRoots>) -> Result<Self, StartupError> {
        roots
            .revalidate()
            .map_err(|_| StartupError::Configuration)?;
        let broker =
            FsVaultBroker::open_existing(roots.vault_directory(), roots.writer_data_directory())
                .map_err(|_| StartupError::Writer)?;
        broker
            .validate_root_identities(roots.vault_identity(), roots.writer_data_identity())
            .map_err(|_| StartupError::Writer)?;
        roots
            .revalidate()
            .map_err(|_| StartupError::Configuration)?;
        Ok(Self { broker, roots })
    }

    pub(super) async fn prepare(self) -> Result<Arc<CanonicalWorkbench>, StartupError> {
        CanonicalWorkbench::prepare(self.broker, self.roots)
            .await
            .map_err(|_| StartupError::Writer)
    }
}

pub(super) async fn shutdown(
    workbench: Option<&Arc<CanonicalWorkbench>>,
) -> Result<(), StartupError> {
    if let Some(workbench) = workbench {
        workbench.stop();
        workbench
            .shutdown()
            .await
            .map_err(|_| StartupError::Writer)?;
    }
    Ok(())
}
