use super::*;
use std::sync::{Arc, mpsc};

#[test]
fn freeze_during_probe_prevents_stale_unfreeze_commit() {
    let admission = Admission::default();
    let frozen = AtomicBool::new(true);
    let epoch = admission.probe_epoch().unwrap();
    admission.invalidate();
    assert!(
        admission
            .unfreeze(
                epoch,
                || {
                    frozen.store(false, Ordering::SeqCst);
                    Ok(())
                },
                || Ok(())
            )
            .is_err()
    );
    assert!(frozen.load(Ordering::SeqCst));
}
#[test]
fn freeze_waits_for_entered_start_and_rejects_later_spawns() {
    let admission = Arc::new(Admission::default());
    let frozen = Arc::new(AtomicBool::new(false));
    let start = admission.start(&frozen).unwrap();
    let (ready, received) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let other = admission.clone();
    let other_frozen = frozen.clone();
    let thread = std::thread::spawn(move || {
        other.invalidate();
        other_frozen.store(true, Ordering::SeqCst);
        ready.send(()).unwrap();
        let _freeze = other.lock().unwrap();
        other_frozen.store(true, Ordering::SeqCst);
        done.send(()).unwrap();
    });
    received.recv().unwrap();
    assert!(matches!(
        finished.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    // 模拟已获准入的派生先行结束，冻结随后才能确认完成。
    drop(start);
    finished.recv().unwrap();
    thread.join().unwrap();
    assert!(admission.start(&frozen).is_err());
}
#[test]
fn shutdown_is_permanent_even_with_old_probe_or_stale_frozen_flag() {
    let admission = Admission::default();
    let epoch = admission.probe_epoch().unwrap();
    admission.close();
    assert!(admission.unfreeze(epoch, || Ok(()), || Ok(())).is_err());
    assert!(admission.probe_epoch().is_err());
    assert!(admission.start(&AtomicBool::new(false)).is_err());
}
#[test]
fn freeze_arriving_during_unfreeze_commit_forces_rollback() {
    let admission = Admission::default();
    let frozen = AtomicBool::new(true);
    let epoch = admission.probe_epoch().unwrap();
    assert!(
        admission
            .unfreeze(
                epoch,
                || {
                    admission.invalidate();
                    frozen.store(false, Ordering::SeqCst);
                    Ok(())
                },
                || {
                    frozen.store(true, Ordering::SeqCst);
                    Ok(())
                }
            )
            .is_err()
    );
    assert!(frozen.load(Ordering::SeqCst));
}

#[test]
fn freeze_commit_also_invalidates_a_probe_started_while_freeze_waited() {
    let admission = Admission::default();
    admission.invalidate();
    let epoch = admission.probe_epoch().unwrap();
    {
        let _freeze = admission.lock().unwrap();
        admission.invalidate();
    }
    assert!(admission.unfreeze(epoch, || Ok(()), || Ok(())).is_err());
}
