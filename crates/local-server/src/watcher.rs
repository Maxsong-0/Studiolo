//! notify 只报告失效；正文始终经 Broker 捕获，回调不扫描或关闭自身。

use std::{path::Path, sync::Arc};

use notify::{
    Event, EventKind, RecursiveMode, Watcher,
    event::{AccessKind, AccessMode, CreateKind, DataChange, MetadataKind, ModifyKind},
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
        Ok(event) if private_stage_event(core.roots.vault_directory(), &event) => {}
        Ok(event) if content_event(&event) => core.watcher_event(generation, "changed", false),
        Ok(_) => {
            #[cfg(test)]
            core.probe.ignored_read();
        }
        Err(_) => core.watcher_event(generation, "watcher_failed", true),
    }
}

// 只屏蔽已知暂存字节事件，不把 notify 路径当身份或安装证明；Broker仍核原stage及根pin。
fn private_stage_event(vault: &Path, event: &Event) -> bool {
    if event.paths.is_empty()
        || !matches!(
            event.kind,
            EventKind::Create(CreateKind::File)
                | EventKind::Modify(ModifyKind::Metadata(
                    MetadataKind::Extended | MetadataKind::Any
                ))
                | EventKind::Modify(ModifyKind::Data(DataChange::Content | DataChange::Any))
                | EventKind::Access(AccessKind::Close(AccessMode::Write))
        )
    {
        return false;
    }
    // notify 8.2的inotify将IN_MODIFY/IN_ATTRIB映射Data(Any)/Metadata(Any)。
    // EventKind/ModifyKind/Name的Any和Other不在上述闭集，不会被吞掉。
    let staging = vault.join(".workbench/staging");
    event.paths.iter().all(|path| {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        let Some(hash) = name.strip_suffix(".stage") else {
            return false;
        };
        hash.len() == 64
            && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            // 不用Path组件相等：它会折叠`.`或重复分隔符，导致别名被错误忽略。
            && path.as_os_str() == staging.join(name).as_os_str()
    })
}

pub(crate) fn content_event(event: &Event) -> bool {
    !matches!(
        event.kind,
        EventKind::Access(
            AccessKind::Read | AccessKind::Open(_) | AccessKind::Close(AccessMode::Read)
        ) | EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))
    )
}
