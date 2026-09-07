//! 真实TCP/Unix绑定共用生产关闭路径；用oneshot结束，所有等待都有明确边界。

use super::super::prepare_composed;
use super::{Fixture, StartupError, test_support, writer::writer_options};
use crate::options::FactorOptions;
use std::{fs, os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
use studiolo_vault_broker::{BrokerError, FsVaultBroker};

async fn prepared(f: &Fixture, unix: bool) -> (super::super::PreparedServer, std::path::PathBuf) {
    let (mut options, data) = writer_options(f);
    if unix {
        let proxy = f.root.join("proxy");
        fs::create_dir(&proxy).unwrap();
        fs::set_permissions(&proxy, fs::Permissions::from_mode(0o700)).unwrap();
        options.factor = Some(FactorOptions {
            socket: proxy.join("http.sock"),
            replacement_seed_ref: None,
        });
    }
    let result = prepare_composed(
        options,
        |_| async { Ok(test_support::verifier().await) },
        || {
            (
                Some("3132333435363738393031323334353637383930".into()),
                None,
            )
        },
    )
    .await
    .unwrap();
    (result, data)
}

#[tokio::test]
async fn tcp_and_unix_graceful_return_release_the_actual_writer_lease() {
    for unix in [false, true] {
        let f = Fixture::new(true);
        let (prepared, data) = prepared(&f, unix).await;
        let workbench = Arc::clone(prepared.workbench.as_ref().unwrap());
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let mut server = tokio::spawn(prepared.serve_until(async move {
            let _ = ready_tx.send(());
            let _ = stop_rx.await;
        }));
        let started = tokio::time::timeout(Duration::from_secs(10), ready_rx).await;
        let exclusive = matches!(
            FsVaultBroker::open_existing(&f.root.join("vault"), &data),
            Err(BrokerError::WriterBusy)
        );
        // 所有断言都在自有服务收尾后执行；超时也不丢弃仍运行的JoinHandle。
        let _ = stop_tx.send(());
        let completed = tokio::time::timeout(Duration::from_secs(10), &mut server).await;
        if completed.is_err() {
            server.abort();
            let _ = server.await;
            workbench.stop();
            let _ = tokio::time::timeout(Duration::from_secs(10), workbench.shutdown()).await;
        }
        assert!(
            started.is_ok_and(|value| value.is_ok()),
            "真实监听未进入关闭等待"
        );
        assert!(exclusive);
        completed.unwrap().unwrap().unwrap();
        drop(workbench);
        drop(FsVaultBroker::open_existing(&f.root.join("vault"), &data).unwrap());
        f.unbound();
        assert!(!f.root.join("proxy/http.sock").exists());
    }
}

#[tokio::test]
async fn tcp_and_unix_bind_failure_also_close_the_writer() {
    for unix in [false, true] {
        let f = Fixture::new(true);
        let (prepared, data) = prepared(&f, unix).await;
        let workbench = Arc::clone(prepared.workbench.as_ref().unwrap());
        let occupied = if unix {
            fs::write(f.root.join("proxy/http.sock"), b"preexisting").unwrap();
            None
        } else {
            Some(std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, f.port)).unwrap())
        };
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            prepared.serve_until(std::future::pending()),
        )
        .await;
        if result.is_err() {
            workbench.stop();
            let _ = tokio::time::timeout(Duration::from_secs(10), workbench.shutdown()).await;
        }
        drop(occupied);
        assert!(matches!(
            result.unwrap(),
            Err(StartupError::Listen | StartupError::Ingress)
        ));
        drop(workbench);
        drop(FsVaultBroker::open_existing(&f.root.join("vault"), &data).unwrap());
        f.unbound();
        if unix {
            assert_eq!(
                fs::read(f.root.join("proxy/http.sock")).unwrap(),
                b"preexisting"
            );
        }
    }
}
