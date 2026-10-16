use std::sync::{Arc, atomic::Ordering};

use studiolo_core::VaultPath;
use studiolo_vault_broker::{Mutation, VaultWriter};

use super::fixture::{Fixture, entered, idle};

#[tokio::test]
async fn cancellation_keeps_the_actual_owner_and_coalesces_one_pending_job() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    let before = workbench.core.probe.captures.load(Ordering::SeqCst);
    let (observed, release) = workbench.core.probe.arm("capture");
    let waiting = Arc::clone(&workbench);
    let request = tokio::spawn(async move { waiting.refresh().await });
    entered(observed).await;
    request.abort();
    assert!(matches!(request.await, Err(error) if error.is_cancelled()));
    assert!(
        workbench.core.owner.try_lock().is_err(),
        "捕获未持有实际 Broker owner"
    );
    for _ in 0..50 {
        workbench.core.request("refreshing").unwrap();
    }
    assert_eq!(
        workbench.core.probe.captures.load(Ordering::SeqCst),
        before + 1
    );
    assert!(workbench.core.control.lock().unwrap().pending);
    release.send(()).unwrap();
    idle(&workbench).await;
    assert_eq!(
        workbench.core.probe.captures.load(Ordering::SeqCst),
        before + 2
    );
    assert!(
        !workbench
            .core
            .view
            .snapshot()
            .unwrap()
            .freshness
            .unwrap()
            .stale
    );
    workbench.shutdown().await.unwrap();
    fixture.available();
}

#[tokio::test]
async fn shutdown_waits_for_the_real_capture_before_releasing_the_lease() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    let (observed, release) = workbench.core.probe.arm("capture");
    workbench.core.request("refreshing").unwrap();
    entered(observed).await;
    workbench.stop();
    assert!(workbench.core.request("refreshing").is_err());
    assert!(
        studiolo_vault_broker::FsVaultBroker::open_existing(
            &fixture.root.join("vault"),
            &fixture.root.join("data")
        )
        .is_err()
    );
    let closing = Arc::clone(&workbench);
    let shutdown = tokio::spawn(async move { closing.shutdown().await });
    assert!(!shutdown.is_finished());
    release.send(()).unwrap();
    shutdown.await.unwrap().unwrap();
    fixture.available();
}

#[tokio::test]
async fn final_revalidation_rejects_invalidation_after_a_successful_publication() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    workbench.revalidate().unwrap();
    let (observed, release) = workbench.core.probe.arm("building");
    workbench.core.request("changed").unwrap();
    entered(observed).await;
    assert!(workbench.revalidate().is_err());
    workbench.stop();
    release.send(()).unwrap();
    workbench.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_new_commit_after_build_invalidates_the_candidate_before_publication() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    let before = workbench.core.view.snapshot().unwrap().generation;
    let (observed, release) = workbench.core.probe.arm("built");
    workbench.core.request("refreshing").unwrap();
    entered(observed).await;
    {
        let mut owner = workbench.core.owner.lock().unwrap();
        let broker = owner.as_mut().unwrap();
        let path = VaultPath::parse("note.md").unwrap();
        let old = broker.read_file(&path, 4096).unwrap();
        workbench.core.request("changed").unwrap();
        broker
            .mutate(Mutation::replace(
                path,
                "# 新提交\n".as_bytes().to_vec(),
                old.revision,
                "fixture-commit",
            ))
            .unwrap();
    }
    release.send(()).unwrap();
    idle(&workbench).await;
    let after = workbench.core.view.snapshot().unwrap();
    assert_eq!(
        after.generation.number,
        before.number + 1,
        "过时候选也被发布了"
    );
    assert_eq!(
        after.freshness.as_ref().unwrap().indexed_through.as_deref(),
        Some("1")
    );
    assert!(!after.freshness.unwrap().stale);
    workbench.shutdown().await.unwrap();
}

#[tokio::test]
async fn every_capture_runs_on_the_dedicated_derivation_thread() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    workbench.refresh().await.unwrap();
    workbench.core.request("refreshing").unwrap();
    idle(&workbench).await;
    let threads = workbench.core.probe.capture_threads();
    assert!(
        threads.len() >= 3,
        "首扫、刷新与请求至少各捕获一次：{threads:?}"
    );
    assert!(
        threads
            .iter()
            .all(|name| name.as_deref() == Some(crate::deriver::THREAD_NAME)),
        "整库捕获与建模必须都在专用派生线程上：{threads:?}"
    );
    workbench.shutdown().await.unwrap();
    fixture.available();
}

#[tokio::test]
async fn a_lost_derivation_thread_fails_closed_without_taking_the_held_owner() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    workbench.core.deriver.lose();
    // 正文保存与 Bridge 写入都在持 owner 锁时请求刷新；投递失败的收尾若再去拿这把锁就会死锁。
    let ticket = {
        let _owner = workbench.core.owner.lock().unwrap();
        workbench.core.request("changed").unwrap()
    };
    let waited = tokio::time::timeout(super::TIMEOUT, workbench.core.wait(ticket))
        .await
        .expect("作业永远不会跑，等待者必须拿到结论而不是挂住");
    assert!(waited.is_err(), "投递失败不能被当成刷新成功");
    assert!(
        workbench.core.request("refreshing").is_err(),
        "失败关闭后不再受理刷新"
    );
    let freshness = workbench.core.view.snapshot().unwrap().freshness.unwrap();
    assert!(freshness.stale);
    assert_eq!(freshness.reason, Some("unavailable"));
    tokio::time::timeout(super::TIMEOUT, workbench.shutdown())
        .await
        .expect("关停不能等一个永远不会跑的作业")
        .unwrap();
    // Broker 租约随协调器析构释放。
    drop(workbench);
    fixture.available();
}
