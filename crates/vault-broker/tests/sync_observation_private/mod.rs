//! 所有故障仅在测试线程、本次 /tmp 夹具触发；生产没有故障开关或回调。
mod boundaries;
mod drift;
mod facts;
mod limits;
mod local_boundaries;
mod local_drift;
mod local_large;
mod two_phase;
use super::*;
use crate::{Mutation, VaultWriter};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::PathBuf,
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        Self {
            root: temp.path().join("vault"),
            data: temp.path().join("data"),
            _temp: temp,
        }
    }
    fn broker(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.root, &self.data).unwrap()
    }
    fn reopen(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.root, &self.data).unwrap()
    }
    fn write(&self, name: &str, body: &[u8]) {
        let target = self.root.join(name);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, body).unwrap();
    }
}
fn path(raw: &str) -> VaultPath {
    VaultPath::parse(raw).unwrap()
}
fn selection() -> SyncReadSelection {
    SyncReadSelection {
        folders: vec![SyncReadRule {
            path: path("notes"),
            max_bytes: 256 * 1024 * 1024,
        }],
        files: vec![],
        excluded_prefixes: vec![path("notes/generated.md"), path("notes/private")],
    }
}
fn capture(broker: &mut FsVaultBroker) -> CanonicalSyncObservation {
    broker.capture_sync_observation(&selection(), &[]).unwrap()
}
fn present<'a>(value: &'a CanonicalSyncObservation, name: &str) -> &'a CanonicalSyncFile {
    match &value.entries()[&path(name)] {
        CanonicalSyncPresence::Present(value) => value,
        _ => panic!("expected Present"),
    }
}
type Fault = (usize, Box<dyn FnOnce()>);
thread_local! {
    static FAULT: RefCell<Option<Fault>> = const { RefCell::new(None) };
    static COUNTS: Cell<[usize;5]> = const { Cell::new([0;5]) };
}
pub(crate) fn hook(phase: usize) {
    COUNTS.with(|counts| {
        let mut values = counts.get();
        values[phase] += 1;
        counts.set(values);
    });
    let fault = FAULT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|(expected, _)| *expected == phase)
        {
            slot.take()
        } else {
            None
        }
    });
    if let Some((_, fault)) = fault {
        fault();
    }
}
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        FAULT.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}
fn fault(phase: usize, f: impl FnOnce() + 'static) -> Guard {
    FAULT.with(|slot| assert!(slot.borrow_mut().replace((phase, Box::new(f))).is_none()));
    Guard
}
fn reset() {
    COUNTS.with(|counts| counts.set([0; 5]));
}
fn count(phase: usize) -> usize {
    COUNTS.with(|counts| counts.get()[phase])
}
