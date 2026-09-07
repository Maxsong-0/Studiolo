use std::{fs, sync::atomic::Ordering};

use notify::{
    Event, EventKind,
    event::{AccessKind, AccessMode, DataChange, Flag, ModifyKind},
};

use super::{
    TIMEOUT,
    fixture::{Fixture, entered, idle},
};
use crate::{
    coordinator::WatchHealth,
    watcher::{content_event, dispatch},
};

#[test]
fn opening_a_file_does_not_claim_a_mutation_but_write_close_does() {
    for mode in [AccessMode::Any, AccessMode::Read, AccessMode::Write] {
        assert!(!content_event(&Event::new(EventKind::Access(
            AccessKind::Open(mode)
        ))));
    }
    assert!(!content_event(&Event::new(EventKind::Access(
        AccessKind::Read
    ))));
    assert!(content_event(&Event::new(EventKind::Access(
        AccessKind::Close(AccessMode::Write)
    ))));
    assert!(content_event(&Event::new(EventKind::Modify(
        ModifyKind::Data(DataChange::Content)
    ))));
}

#[tokio::test]
async fn rescan_requires_a_complete_capture_and_backend_error_requires_reregistration() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    for failed in [false, true] {
        let before = workbench.core.control.lock().unwrap().watch_generation;
        let captures = workbench.core.probe.captures.load(Ordering::SeqCst);
        let (observed, release) = workbench.core.probe.arm("capture");
        if failed {
            dispatch(
                &workbench.core,
                before,
                Err(notify::Error::generic("synthetic watcher error")),
            );
        } else {
            dispatch(
                &workbench.core,
                before,
                Ok(
                    Event::new(EventKind::Access(AccessKind::Open(AccessMode::Any)))
                        .set_flag(Flag::Rescan),
                ),
            );
        }
        entered(observed).await;
        assert!(
            workbench
                .core
                .view
                .snapshot()
                .unwrap()
                .freshness
                .unwrap()
                .stale
        );
        release.send(()).unwrap();
        idle(&workbench).await;
        let state = workbench.core.control.lock().unwrap();
        assert!(state.health == WatchHealth::Healthy);
        assert_eq!(state.watch_generation, before + u64::from(failed));
        drop(state);
        assert_eq!(
            workbench.core.probe.captures.load(Ordering::SeqCst),
            captures + 1
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
    }
    workbench.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_real_notify_change_rebuilds_the_canonical_view() {
    let fixture = Fixture::new();
    let workbench = fixture.start(true).await;
    let before = workbench.core.view.snapshot().unwrap().generation.number;
    let mut updates = workbench.core.completion.subscribe();
    fs::write(
        fixture.root.join("vault/added.md"),
        "# 实际通知的虚构新页\n",
    )
    .unwrap();
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let _ = updates.borrow_and_update();
            let view = workbench.core.view.snapshot().unwrap();
            if view.generation.number > before
                && !view.freshness.unwrap().stale
                && view
                    .generation
                    .model
                    .document(&studiolo_core::VaultPath::parse("added.md").unwrap())
                    .is_some()
            {
                break;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .expect("真实 notify 未完成变化后的整库发布");
    workbench.shutdown().await.unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn actual_inotify_read_events_do_not_schedule_another_capture() {
    let fixture = Fixture::new();
    let workbench = fixture.start(true).await;
    let before = workbench.core.control.lock().unwrap().requested;
    let observed = workbench.core.probe.observe_read();
    workbench
        .read_file(&studiolo_core::VaultPath::parse("note.md").unwrap(), 4096)
        .unwrap();
    entered(observed).await;
    assert_eq!(workbench.core.control.lock().unwrap().requested, before);
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
}
