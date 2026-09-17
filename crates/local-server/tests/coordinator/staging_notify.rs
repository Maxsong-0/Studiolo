//! 真正notify回调接入同一dispatch；在有界事件到达点核对stage与公开来源的区别。
#![cfg(any(target_os = "macos", target_os = "linux"))]
use super::{
    TIMEOUT,
    fixture::{Fixture, entered, idle},
};
use crate::watcher::dispatch;
use notify::{
    EventKind, RecursiveMode, Watcher,
    event::{DataChange, ModifyKind},
};
use std::{fs, sync::Arc};

// 两平台必须看到实际内容通知；不能只靠超时内没有刷新宣称过滤有效。
#[tokio::test]
async fn actual_notify_stage_write_preserves_epoch_but_public_write_invalidates() {
    let f = Fixture::new();
    // 关闭夹具内原回调，让下面真实推荐backend成为唯一送入dispatch的事件来源。
    let w = f.start(false).await;
    let vault = f.root.join("vault");
    let stage = vault.join(format!(".workbench/staging/{}.stage", "e".repeat(64)));
    let source = vault.join("note.md");
    let (send, mut receive) = tokio::sync::mpsc::channel(64);
    let core = Arc::clone(&w.core);
    let generation = core.control.lock().unwrap().watch_generation;
    let tracked_stage = stage.clone();
    let tracked_source = source.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let inspected = event.as_ref().ok().map(|event| {
            (
                event.kind,
                event.paths.contains(&tracked_stage),
                event.paths.contains(&tracked_source),
            )
        });
        dispatch(&core, generation, event);
        if let Some((kind, stage, source)) = inspected
            && (stage || source)
            && matches!(
                kind,
                EventKind::Modify(ModifyKind::Data(DataChange::Content | DataChange::Any))
            )
        {
            let epoch = core.control.lock().unwrap().epoch;
            send.try_send((stage, source, epoch))
                .expect("合成通知超出有界夹具容量");
        }
    })
    .unwrap();
    watcher.watch(&vault, RecursiveMode::Recursive).unwrap();
    let before = w.core.control.lock().unwrap().epoch;
    fs::write(&stage, b"synthetic staged bytes").unwrap();
    tokio::time::timeout(TIMEOUT, async {
        while let Some((is_stage, _, epoch)) = receive.recv().await {
            if is_stage {
                assert_eq!(epoch, before);
                return;
            }
        }
        panic!("真实stage事件通道提前关闭");
    })
    .await
    .expect("没有真实stage内容事件，不能以空等待冒充通过");
    assert!(!w.core.view.snapshot().unwrap().freshness.unwrap().stale);
    // 暂停实际后续capture，避免后台刷新把刚收到的公开失效提前清除。
    let (observed, release) = w.core.probe.arm("capture");
    fs::write(&source, "# 公开来源必须失效\n").unwrap();
    tokio::time::timeout(TIMEOUT, async {
        while let Some((_, is_source, epoch)) = receive.recv().await {
            if is_source {
                assert!(epoch > before);
                return;
            }
        }
        panic!("真实source事件通道提前关闭");
    })
    .await
    .expect("公开来源变化没有触发真实notify失效");
    entered(observed).await;
    assert!(w.core.view.snapshot().unwrap().freshness.unwrap().stale);
    release.send(()).unwrap();
    drop(watcher);
    fs::remove_file(stage).unwrap();
    idle(&w).await;
    w.shutdown().await.unwrap();
}
