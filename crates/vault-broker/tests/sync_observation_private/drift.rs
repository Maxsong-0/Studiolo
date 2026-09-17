use super::*;
use std::sync::atomic::Ordering;

#[test]
fn metadata_to_body_changes_do_not_publish_partial_or_missing_results() {
    for attack in 0..5 {
        let f = Fixture::new();
        let mut broker = f.broker();
        f.write("notes/a", b"original");
        let root = f.root.clone();
        let _fault = fault(0, move || match attack {
            0 => fs::write(root.join("notes/new"), b"new").unwrap(),
            1 => fs::write(root.join("notes/a"), b"modified").unwrap(),
            2 => {
                fs::rename(root.join("notes"), root.join("moved")).unwrap();
                fs::create_dir(root.join("notes")).unwrap();
            }
            3 => {
                fs::remove_file(root.join("notes/a")).unwrap();
                std::os::unix::fs::symlink("/etc/passwd", root.join("notes/a")).unwrap();
            }
            _ => {
                fs::rename(&root, root.with_extension("old")).unwrap();
                fs::create_dir(&root).unwrap();
            }
        });
        assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
    }
}

#[test]
fn same_content_inode_replacement_and_new_missing_target_invalidate_old_proof() {
    for missing in [false, true] {
        let f = Fixture::new();
        let mut broker = f.broker();
        f.write("notes/a", b"same");
        let result = broker
            .capture_sync_observation(&selection(), &[path("notes/new")])
            .unwrap();
        if missing {
            f.write("notes/new", b"new");
        } else {
            f.write("notes/temporary", b"same");
            fs::rename(f.root.join("notes/temporary"), f.root.join("notes/a")).unwrap();
        }
        assert!(broker.revalidate_sync_observation(&result).is_err());
    }
}

#[test]
fn mutation_during_first_chunk_or_final_publication_is_rejected() {
    for phase in [2, 3] {
        let f = Fixture::new();
        let mut broker = f.broker();
        f.write("notes/a", &vec![b'a'; 131072]);
        let target = f.root.join("notes/a");
        let _fault = fault(phase, move || {
            let file = fs::OpenOptions::new().write(true).open(target).unwrap();
            file.set_len(9).unwrap();
        });
        assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
    }
}

#[test]
fn opened_directory_must_match_current_path_before_enumerating() {
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/a", b"old");
    let root = f.root.clone();
    let _fault = fault(4, move || {
        fs::rename(root.join("notes"), root.join("previous")).unwrap();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join("notes/new"), b"new").unwrap();
    });
    reset();
    assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
    assert_eq!(count(1), 0);
}

#[test]
fn source_growth_never_extends_the_prevalidated_actual_body_budget() {
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/a", &vec![b'a'; 65536]);
    let target = f.root.join("notes/a");
    let _fault = fault(3, move || {
        fs::OpenOptions::new()
            .write(true)
            .open(target)
            .unwrap()
            .set_len(256 * 1024 * 1024)
            .unwrap();
    });
    reset();
    assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
    assert_eq!(count(3), 1);
}

#[test]
fn proof_rejects_reopen_other_instance_and_committed_watermark_advance() {
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/a", b"before");
    let proof = capture(&mut broker);
    drop(broker);
    let mut reopened = f.reopen();
    assert!(reopened.revalidate_sync_observation(&proof).is_err());
    let fresh = capture(&mut reopened);
    reopened
        .mutate(Mutation::create(
            path("notes/new"),
            b"new".to_vec(),
            "sync-proof-watermark",
        ))
        .unwrap();
    assert!(reopened.revalidate_sync_observation(&fresh).is_err());
    let other = Fixture::new();
    let different = other.broker();
    assert!(different.revalidate_sync_observation(&fresh).is_err());
}

#[test]
fn read_blocked_installing_and_poisoned_writer_reject_both_capture_and_revalidation() {
    for point in [2, 3, 4, 5] {
        let f = Fixture::new();
        let mut broker = f.broker();
        f.write("notes/a", b"readable");
        let proof = capture(&mut broker);
        broker.writer.fault.store(point, Ordering::Release);
        assert!(
            broker
                .mutate(Mutation::create(
                    path("notes/new"),
                    b"new".to_vec(),
                    format!("sync-unknown-{point}")
                ))
                .is_err()
        );
        reset();
        assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
        assert_eq!(count(1), 0);
        assert!(broker.revalidate_sync_observation(&proof).is_err());
    }
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/a", b"readable");
    let proof = capture(&mut broker);
    broker.writer.poison();
    assert!(broker.revalidate_sync_observation(&proof).is_err());
    assert!(broker.capture_sync_observation(&selection(), &[]).is_err());
}
