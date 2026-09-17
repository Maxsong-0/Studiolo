//! 私有暂存例外不能吞掉歧义、公开路径或身份事件。
use super::fixture::{Fixture, idle};
use crate::watcher::dispatch;
use notify::{
    Event, EventKind,
    event::{
        AccessKind, CreateKind, DataChange, Flag, MetadataKind, ModifyKind, RemoveKind, RenameMode,
    },
};
use std::path::PathBuf;

fn content(path: PathBuf) -> Event {
    Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content))).add_path(path)
}

#[tokio::test]
async fn ambiguous_alias_mixed_and_private_identity_events_always_invalidate() {
    let f = Fixture::new();
    let w = f.start(false).await;
    let vault = f.root.join("vault");
    let name = format!("{}.stage", "a".repeat(64));
    let stage = vault.join(".workbench/staging").join(&name);
    let kinds = [
        EventKind::Any,
        EventKind::Other,
        EventKind::Modify(ModifyKind::Any),
        EventKind::Modify(ModifyKind::Data(DataChange::Other)),
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::Other)),
        EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
        EventKind::Modify(ModifyKind::Name(RenameMode::Other)),
        EventKind::Create(CreateKind::Any),
        EventKind::Create(CreateKind::Folder),
        EventKind::Create(CreateKind::Other),
        EventKind::Access(AccessKind::Any),
        EventKind::Remove(RemoveKind::File),
    ];
    let mut events: Vec<_> = kinds
        .into_iter()
        .map(|kind| Event::new(kind).add_path(stage.clone()))
        .collect();
    events.extend([
        Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content))),
        content(stage.clone()).set_flag(Flag::Rescan),
        content(stage.clone()).add_path(vault.join("note.md")),
        Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(stage.clone())
            .add_path(vault.join("note.md")),
    ]);
    for path in [
        vault.clone(),
        vault.join(".workbench"),
        vault.join(".workbench/staging"),
        vault.join(".workbench/vault.json"),
        vault.join(".workbench/broker-binding.json"),
        vault.join(".workbench/deleted-objects").join(&name),
        vault.join(".workbench/staging/sub").join(&name),
        vault.join(".workbench/staging").join(name.to_uppercase()),
        vault.join(".workbench/staging/bad.stage"),
        PathBuf::from(format!("{}/.workbench/staging/./{name}", vault.display())),
        PathBuf::from(format!("{}/.workbench/staging//{name}", vault.display())),
        vault.join(".workbench/staging/../staging").join(&name),
        vault.join(".WORKBENCH/staging").join(&name),
        PathBuf::from(format!(".workbench/staging/{name}")),
    ] {
        events.push(content(path));
    }
    // owner仅在测试中阻止后台捕获抢先发布；dispatch不能反向取得owner。
    {
        let _owner = w.core.owner.lock().unwrap();
        for event in events {
            let (generation, epoch) = {
                let state = w.core.control.lock().unwrap();
                (state.watch_generation, state.epoch)
            };
            dispatch(&w.core, generation, Ok(event));
            assert_eq!(w.core.control.lock().unwrap().epoch, epoch + 1);
            assert!(w.core.view.snapshot().unwrap().freshness.unwrap().stale);
        }
        let (generation, epoch) = {
            let state = w.core.control.lock().unwrap();
            (state.watch_generation, state.epoch)
        };
        dispatch(
            &w.core,
            generation,
            Err(notify::Error::generic("synthetic stage watcher error")),
        );
        assert_eq!(w.core.control.lock().unwrap().epoch, epoch + 1);
    }
    idle(&w).await;
    w.shutdown().await.unwrap();
}
