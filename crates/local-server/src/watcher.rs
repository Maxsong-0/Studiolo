//! notify 只报告失效；正文始终经 Broker 捕获，回调不扫描或关闭自身。

use std::sync::Arc;

use notify::{
    Event, EventKind, RecursiveMode, Watcher,
    event::{AccessKind, AccessMode, MetadataKind, ModifyKind},
};

use crate::coordinator::{Core, Failure, WatchHealth};

pub(crate) fn ensure(core: &Arc<Core>) -> Result<(), Failure> {
    let generation = {
        let mut state = core.control.lock().map_err(|_| Failure::Internal)?;
        if state.closing {
            return Err(Failure::Stopped);
        }
        if state.health == WatchHealth::Healthy {
            return Ok(());
        }
        state.watch_generation = state
            .watch_generation
            .checked_add(1)
            .ok_or(Failure::Internal)?;
        state.health = WatchHealth::Starting;
        state.watch_generation
    };
    let old = core.watcher.lock().map_err(|_| Failure::Internal)?.take();
    drop(old);
    core.roots.revalidate().map_err(|_| Failure::Binding)?;
    let weak = Arc::downgrade(core);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        let Some(core) = weak.upgrade() else {
            return;
        };
        #[cfg(test)]
        if !core
            .probe
            .os_events
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return;
        }
        dispatch(&core, generation, event);
    })
    .map_err(|_| Failure::Watcher)?;
    watcher
        .watch(core.roots.vault_directory(), RecursiveMode::Recursive)
        .map_err(|_| Failure::Watcher)?;
    let mut state = core.control.lock().map_err(|_| Failure::Internal)?;
    if state.closing {
        return Err(Failure::Stopped);
    }
    if state.health != WatchHealth::Starting || state.watch_generation != generation {
        return Err(Failure::Watcher);
    }
    *core.watcher.lock().map_err(|_| Failure::Internal)? = Some(watcher);
    state.health = WatchHealth::Healthy;
    Ok(())
}

pub(crate) fn dispatch(core: &Arc<Core>, generation: u64, event: notify::Result<Event>) {
    match event {
        Ok(event) if event.need_rescan() => core.watcher_event(generation, "watcher_rescan", false),
        Ok(event) if content_event(&event) => core.watcher_event(generation, "changed", false),
        Ok(_) => {
            #[cfg(test)]
            core.probe.ignored_read();
        }
        Err(_) => core.watcher_event(generation, "watcher_failed", true),
    }
}

pub(crate) fn content_event(event: &Event) -> bool {
    !matches!(
        event.kind,
        EventKind::Access(
            AccessKind::Read | AccessKind::Open(_) | AccessKind::Close(AccessMode::Read)
        ) | EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))
    )
}
