use super::*;
use std::{fs, os::unix::fs::PermissionsExt, sync::atomic::Ordering};
use studiolo_backup::{store::storage::BackupStore, worker::state::UnknownStage};

#[test]
fn actual_private_store_reopen_preserves_ack_unknown_and_finishes_after_the_original_source_is_gone()
 {
    let world = World::new();
    let root = world._fixture.temp.path().join("worker-state");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let store = BackupStore::open(&root, true).unwrap();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2000);
    *world.finalizer.lose_ack.lock().unwrap() = true;
    let worker = BackupWorker::new(
        &world.ledger,
        &store,
        repository(),
        CredentialHolder::BackupWorker,
        BackupTargetKind::LocalTemporaryRepository,
    )
    .unwrap();
    let key = worker.enqueue(&checkpoint()).unwrap();
    assert_eq!(
        worker
            .run(
                &key,
                &world.seal,
                DataRemote {
                    runner: &runner,
                    probe: &probe
                },
                &world.finalizer
            )
            .unwrap(),
        JobOutcome::ResultUnknown(UnknownStage::FinalManifestAck)
    );
    let original = store.load_state().unwrap().unwrap();
    // 仅移动本夹具原封存，使原路径消失；固定待提交记录无需重新打开它。
    let sealed_root = &world.seal.ticket().transfer_source.root;
    fs::set_permissions(sealed_root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::rename(
        sealed_root,
        world._fixture.temp.path().join("expired-source"),
    )
    .unwrap();
    assert!(!sealed_root.exists());
    drop(store);
    let reopened = BackupStore::open(&root, false).unwrap();
    assert_eq!(reopened.load_state().unwrap().unwrap(), original);
    let worker = BackupWorker::new(
        &world.ledger,
        &reopened,
        repository(),
        CredentialHolder::BackupWorker,
        BackupTargetKind::LocalTemporaryRepository,
    )
    .unwrap();
    assert_eq!(
        worker.reconcile_final(&key, &world.finalizer).unwrap(),
        JobOutcome::Succeeded
    );
    let finished = reopened.load_state().unwrap().unwrap();
    assert_eq!(
        finished.last_final.as_ref().unwrap().draft(),
        &original.pending_final.unwrap().draft
    );
    assert_eq!(world.finalizer.commits.load(Ordering::SeqCst), 1);
    assert_eq!(runner.seen.lock().unwrap().len(), 1);
    assert_eq!(probe.probed.lock().unwrap().len(), 1);
    drop(reopened);
    let second_reopen = BackupStore::open(&root, false).unwrap();
    assert_eq!(second_reopen.load_state().unwrap().unwrap(), finished);
}
