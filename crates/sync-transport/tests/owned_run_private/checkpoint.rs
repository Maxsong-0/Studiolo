//! 真实文件导出与脚本化运行状态机；不冒充真实 cgroup 平台验收。
use super::support::*;
use crate::owned_run::{
    checkpoint::{candidates, snapshot},
    *,
};
use crate::store::{FilePublisher, StatePublisher};
use std::{
    fs,
    os::unix::fs::{MetadataExt as _, PermissionsExt as _},
    time::{Duration, Instant},
};
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

#[test]
fn checkpoint_terminal_events_match_original_candidates_and_active_states_refuse() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    assert!(
        snapshot(&session, deadline())
            .unwrap()
            .last_terminal
            .is_none()
    );
    let prepared = session.prepare_once(1, RunOperation::RunOnce, 1).unwrap();
    assert_eq!(
        snapshot(&session, deadline()).unwrap_err(),
        TransportError::RunActive
    );
    let intent = session.pending_audit().unwrap().unwrap();
    acknowledge(&mut session);
    let owned = session.start(&prepared).unwrap();
    assert_eq!(
        snapshot(&session, deadline()).unwrap_err(),
        TransportError::RunActive
    );
    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = false;
    session.wait(&owned).unwrap();
    let outcome = session.pending_audit().unwrap().unwrap();
    assert_eq!(
        snapshot(&session, deadline()).unwrap_err(),
        TransportError::RunActive
    );
    acknowledge(&mut session);
    let snapshot = snapshot(&session, deadline()).unwrap();
    let expected = candidates(&session, &snapshot).unwrap();
    for (before, after) in [&intent, &outcome].into_iter().zip(&expected) {
        assert_eq!(before.event_id(), after.event_id());
        assert_eq!(before.phase(), after.phase());
        assert_eq!(before.payload(), after.payload());
    }
}
#[test]
fn checkpoint_unproven_idle_and_expired_window_refuse() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    assert_eq!(
        snapshot(&session, Instant::now()).unwrap_err(),
        TransportError::Budget
    );
    let prepared = session.prepare_once(1, RunOperation::RunOnce, 1).unwrap();
    session.cancel_prepared(prepared).unwrap();
    acknowledge(&mut session);
    acknowledge(&mut session);
    assert_eq!(
        snapshot(&session, deadline()).unwrap_err(),
        TransportError::QuiescenceUnconfirmed
    );
    let mut state = fixture.slot.state();
    state.last_terminal = None;
    fixture.slot.set_bytes(state::encode(&state).unwrap());
    assert_eq!(
        snapshot(&session, deadline()).unwrap_err(),
        TransportError::QuiescenceUnconfirmed
    );
}
#[test]
fn checkpoint_real_publisher_copies_only_fixed_state_and_keeps_original_lease() {
    let root = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    for path in [&source, &target] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let fixture = Fixture::new();
    let bytes = fixture.slot.bytes().unwrap();
    let publisher = FilePublisher::initialize(&source, &bytes).unwrap();
    publisher
        .export_checkpoint(&target, &bytes, deadline())
        .unwrap();
    assert_eq!(
        FilePublisher::open_existing(&source).err(),
        Some(TransportError::Busy)
    );
    let copied = FilePublisher::open_existing(&target).unwrap();
    assert_eq!(copied.read().unwrap(), bytes);
    assert_eq!(
        fs::metadata(target.join("state.json")).unwrap().mode() & 0o777,
        0o600
    );
    assert_ne!(
        fs::metadata(source.join("state.json")).unwrap().ino(),
        fs::metadata(target.join("state.json")).unwrap().ino()
    );
    assert!(
        publisher
            .export_checkpoint(&target, &bytes, deadline())
            .is_err()
    );
    assert!(
        publisher
            .export_checkpoint(&source, &bytes, deadline())
            .is_err()
    );
    drop(publisher);
    assert!(FilePublisher::open_existing(&source).is_ok());
}
#[test]
fn checkpoint_wrong_source_residue_and_timeout_leave_empty_targets() {
    let root = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    for path in [&source, &target] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let fixture = Fixture::new();
    let bytes = fixture.slot.bytes().unwrap();
    let publisher = FilePublisher::initialize(&source, &bytes).unwrap();
    assert!(
        publisher
            .export_checkpoint(&target, b"wrong", deadline())
            .is_err()
    );
    assert!(
        publisher
            .export_checkpoint(&target, &bytes, Instant::now())
            .is_err()
    );
    fs::write(source.join("state.json.publish"), b"interrupted").unwrap();
    fs::set_permissions(
        source.join("state.json.publish"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(
        publisher
            .export_checkpoint(&target, &bytes, deadline())
            .is_err()
    );
    assert!(fs::read_dir(target).unwrap().next().is_none());
}
