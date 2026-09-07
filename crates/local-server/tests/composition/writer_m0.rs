//! 真实M0组合已拥有Broker时不允许第二owner；逆序启动同样受同一根租约约束。

use super::{Fixture, StartupError, prepare_with, test_support};
use std::{fs, os::unix::fs::PermissionsExt, sync::Arc};
use studiolo::{M0OpenError, M0Workbench};
use studiolo_vault_broker::BrokerError;

#[tokio::test]
async fn m0_task_engine_and_canonical_service_cannot_independently_own_the_same_root() {
    let f = Fixture::new(true);
    let vault = f.root.join("vault");
    let data = f.root.join("writer-data");
    fs::create_dir(&data).unwrap();
    fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
    let m0 = M0Workbench::open(&vault, &data).unwrap();
    let mut options = f.options();
    options.writer_data = Some(data.clone());
    assert!(matches!(
        prepare_with(options, |_| async { panic!("M0持租约时不得取钥") }).await,
        Err(StartupError::Writer)
    ));
    f.unbound();
    drop(m0);

    let mut options = f.options();
    options.writer_data = Some(data.clone());
    let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .unwrap();
    assert!(matches!(
        M0Workbench::open(&vault, &data),
        Err(M0OpenError::Broker(BrokerError::WriterBusy))
    ));
    let alternative = f.root.join("alternative-data");
    assert!(matches!(
        M0Workbench::open(&vault, &alternative),
        Err(M0OpenError::Broker(BrokerError::WriterBusy))
    ));
    assert!(!alternative.exists());
    let workbench = Arc::clone(prepared.workbench.as_ref().unwrap());
    workbench.stop();
    workbench.shutdown().await.unwrap();
    drop((prepared, workbench));
    drop(M0Workbench::open(&vault, &data).unwrap());
}
