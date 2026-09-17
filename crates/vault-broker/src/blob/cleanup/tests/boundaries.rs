use super::*;

#[test]
fn malformed_owned_names_reject_both_directories_before_any_deletion() {
    for malformed in [
        ".studiolo-blob-not-a-uuid.tmp",
        ".studiolo-blob-00000000-0000-0000-0000-000000000001.tmp.extra",
        ".studiolo-blob-AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA.tmp",
    ] {
        let f = Fixture::new();
        let mut b = f.open();
        let safe = f.temporary("history", b"safe");
        let bad = f.data.join("snapshots").join(malformed);
        fs::write(&bad, b"unknown").unwrap();
        assert!(matches!(
            b.prune_blob_temporaries(),
            Err(BrokerError::CorruptWriterState)
        ));
        assert_eq!(fs::read(safe).unwrap(), b"safe");
        assert_eq!(fs::read(bad).unwrap(), b"unknown");
    }
}

#[cfg(unix)]
#[test]
fn symlinks_hardlinks_and_special_nodes_are_not_opened_or_removed() {
    for kind in [0, 1, 2, 3] {
        let f = Fixture::new();
        let mut b = f.open();
        let safe = f.temporary("history", b"safe");
        let bad = f.temporary("snapshots", b"old");
        fs::remove_file(&bad).unwrap();
        let external = f._temp.path().join("outside");
        fs::write(&external, b"outside").unwrap();
        let mut socket = None;
        match kind {
            0 => std::os::unix::fs::symlink(&external, &bad).unwrap(),
            1 => fs::hard_link(&external, &bad).unwrap(),
            2 => {
                let short = f._temp.path().join("socket");
                socket = Some(std::os::unix::net::UnixListener::bind(&short).unwrap());
                fs::rename(short, &bad).unwrap();
            }
            _ => fs::create_dir(&bad).unwrap(),
        }
        assert!(b.prune_blob_temporaries().is_err());
        assert!(fs::symlink_metadata(&bad).is_ok());
        assert_eq!(fs::read(&safe).unwrap(), b"safe");
        assert_eq!(fs::read(external).unwrap(), b"outside");
        drop(socket);
    }
}

#[test]
fn replaced_rewritten_or_new_candidate_invalidates_the_whole_sealed_plan() {
    for change in [0, 1, 2] {
        let f = Fixture::new();
        let b = f.open();
        let left = f.temporary("history", b"left");
        let right = f.temporary("snapshots", b"right");
        let plan = prepare(&b).unwrap();
        match change {
            0 => {
                fs::rename(&right, f._temp.path().join("held-original")).unwrap();
                fs::write(&right, b"right").unwrap();
            }
            1 => fs::write(&right, b"other").unwrap(),
            _ => {
                f.temporary("snapshots", b"additional");
            }
        }
        assert!(run::execute(&b, plan).is_err());
        assert_eq!(fs::read(left).unwrap(), b"left");
        assert!(right.exists());
    }
}

#[test]
fn plan_cannot_cross_a_broker_reopen_or_same_content_journal_replacement() {
    for reopen in [false, true] {
        let f = Fixture::new();
        let b = f.open();
        let left = f.temporary("history", b"left");
        let plan = prepare(&b).unwrap();
        if reopen {
            drop(b);
            let b = f.reopen();
            assert!(matches!(
                run::execute(&b, plan),
                Err(BrokerError::CaptureChanged)
            ));
        } else {
            let name = f.data.join("writer-state.json");
            let content = fs::read(&name).unwrap();
            fs::rename(&name, f._temp.path().join("held-journal")).unwrap();
            fs::write(&name, content).unwrap();
            assert!(matches!(
                run::execute(&b, plan),
                Err(BrokerError::CaptureChanged)
            ));
        }
        assert_eq!(fs::read(left).unwrap(), b"left");
    }
}

#[test]
fn absolute_roots_and_private_directory_binding_are_rechecked_before_deletion() {
    for location in ["vault", "data", "history", "snapshots"] {
        let f = Fixture::new();
        let b = f.open();
        let left = f.temporary("history", b"left");
        let right = f.temporary("snapshots", b"right");
        let plan = prepare(&b).unwrap();
        let original = match location {
            "vault" => f.root.clone(),
            "data" => f.data.clone(),
            other => f.data.join(other),
        };
        let held = f._temp.path().join("held-directory");
        fs::rename(&original, &held).unwrap();
        fs::create_dir(&original).unwrap();
        assert!(run::execute(&b, plan).is_err());
        let retained = |name: &std::path::Path| {
            if let Ok(relative) = name.strip_prefix(&original) {
                held.join(relative)
            } else {
                name.to_path_buf()
            }
        };
        assert_eq!(fs::read(retained(&left)).unwrap(), b"left");
        assert_eq!(fs::read(retained(&right)).unwrap(), b"right");
    }
}
