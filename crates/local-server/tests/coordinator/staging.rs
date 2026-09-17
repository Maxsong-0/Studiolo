//! 直接走实际dispatch，限定事件与原样路径；不依赖真实notify的交付时序。
use super::fixture::{Fixture, entered, idle};
use crate::watcher::dispatch;
use notify::{
    Event, EventKind,
    event::{AccessKind, AccessMode, CreateKind, DataChange, MetadataKind, ModifyKind},
};
use std::{fs, path::PathBuf};

fn content(path: PathBuf) -> Event {
    Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content))).add_path(path)
}

#[tokio::test]
async fn only_known_events_on_exact_direct_stage_names_preserve_the_epoch() {
    let f = Fixture::new();
    let w = f.start(false).await;
    let stage = f
        .root
        .join(format!("vault/.workbench/staging/{}.stage", "a".repeat(64)));
    let other = f
        .root
        .join(format!("vault/.workbench/staging/{}.stage", "b".repeat(64)));
    let (generation, epoch, requested) = {
        let state = w.core.control.lock().unwrap();
        (state.watch_generation, state.epoch, state.requested)
    };
    for kind in [
        EventKind::Create(CreateKind::File),
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::Extended)),
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        EventKind::Modify(ModifyKind::Data(DataChange::Any)),
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
        EventKind::Access(AccessKind::Close(AccessMode::Write)),
    ] {
        dispatch(
            &w.core,
            generation,
            Ok(Event::new(kind).add_path(stage.clone())),
        );
        dispatch(
            &w.core,
            generation,
            Ok(Event::new(kind)
                .add_path(stage.clone())
                .add_path(other.clone())),
        );
    }
    {
        let state = w.core.control.lock().unwrap();
        assert_eq!((state.epoch, state.requested), (epoch, requested));
    }
    assert!(!w.core.view.snapshot().unwrap().freshness.unwrap().stale);
    w.shutdown().await.unwrap();
}

#[tokio::test]
async fn public_source_event_still_rejects_a_candidate_built_before_the_change() {
    let f = Fixture::new();
    let w = f.start(false).await;
    let before = w.core.view.snapshot().unwrap().generation.number;
    let (observed, release) = w.core.probe.arm("built");
    w.core.request("refreshing").unwrap();
    entered(observed).await;
    let (generation, epoch) = {
        let state = w.core.control.lock().unwrap();
        (state.watch_generation, state.epoch)
    };
    let stage = f
        .root
        .join(format!("vault/.workbench/staging/{}.stage", "c".repeat(64)));
    dispatch(&w.core, generation, Ok(content(stage)));
    assert_eq!(w.core.control.lock().unwrap().epoch, epoch);
    let source = f.root.join("vault/note.md");
    fs::write(&source, "# 暂存同时发生的公开来源变化\n").unwrap();
    dispatch(&w.core, generation, Ok(content(source)));
    assert_eq!(w.core.control.lock().unwrap().epoch, epoch + 1);
    release.send(()).unwrap();
    idle(&w).await;
    // 过时的第一个候选不发布，只有随后完整捕获会推进一代。
    assert_eq!(
        w.core.view.snapshot().unwrap().generation.number,
        before + 1
    );
    assert!(!w.core.view.snapshot().unwrap().freshness.unwrap().stale);
    w.shutdown().await.unwrap();
}

#[tokio::test]
async fn ignored_child_event_is_not_a_replacement_for_actual_private_root_pins() {
    let f = Fixture::new();
    let w = f.start(false).await;
    let old = f.root.join("vault/.workbench/staging");
    fs::rename(&old, f.root.join("vault/.workbench/staging-old")).unwrap();
    fs::create_dir(&old).unwrap();
    let generation = w.core.control.lock().unwrap().watch_generation;
    dispatch(
        &w.core,
        generation,
        Ok(content(old.join(format!("{}.stage", "d".repeat(64))))),
    );
    assert!(w.revalidate().is_err(), "路径过滤不能授权替换后的private根");
    w.shutdown().await.unwrap();
}
