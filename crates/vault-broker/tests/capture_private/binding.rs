use super::*;

#[test]
fn stamps_are_bound_to_one_instance_without_retaining_its_lease() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    let stamp = capture(&mut broker).stamp;
    assert!(matches!(
        FsVaultBroker::open_existing(&fixture.root, &fixture.data),
        Err(BrokerError::WriterBusy)
    ));
    drop(broker);
    let mut reopened = fixture.open();
    assert!(matches!(
        reopened.revalidate_capture(&stamp),
        Err(BrokerError::CaptureChanged)
    ));
    let fresh = capture(&mut reopened);
    reopened.revalidate_capture(&fresh.stamp).unwrap();
    assert_eq!(fresh.stamp.committed_through(), stamp.committed_through());
    let other = Fixture::new();
    let other_broker = other.initialize();
    assert!(matches!(
        other_broker.revalidate_capture(&fresh.stamp),
        Err(BrokerError::CaptureChanged)
    ));
}

#[test]
fn approved_root_identities_must_match_both_actually_opened_roots() {
    let fixture = Fixture::new();
    let broker = fixture.initialize();
    let canonical = identity(&fixture.root);
    let data = identity(&fixture.data);
    broker.validate_root_identities(canonical, data).unwrap();
    assert!(matches!(
        broker.validate_root_identities(data, canonical),
        Err(BrokerError::WriterBindingMismatch)
    ));
    let other = Fixture::new();
    let _other_broker = other.initialize();
    assert!(matches!(
        broker.validate_root_identities(identity(&other.root), data),
        Err(BrokerError::WriterBindingMismatch)
    ));
    assert!(matches!(
        broker.validate_root_identities(canonical, identity(&other.data)),
        Err(BrokerError::WriterBindingMismatch)
    ));
}

#[test]
fn replacing_any_pinned_root_or_control_object_rejects_all_capture_reads() {
    for node in ["root", "control", "marker", "data", "history"] {
        let fixture = Fixture::new();
        let mut broker = fixture.initialize();
        fs::write(fixture.root.join("note.md"), b"fixture").unwrap();
        let stamp = capture(&mut broker).stamp;
        let approved = (identity(&fixture.root), identity(&fixture.data));
        let target = match node {
            "root" => fixture.root.clone(),
            "control" => fixture.root.join(".workbench"),
            "marker" => fixture.root.join(".workbench/vault.json"),
            "history" => fixture.data.join("history"),
            _ => fixture.data.clone(),
        };
        let moved = fixture.temp.path().join("moved");
        fs::rename(&target, &moved).unwrap();
        if node == "marker" {
            fs::copy(&moved, &target).unwrap();
        } else {
            fs::create_dir(&target).unwrap();
        }
        let before = inventory(fixture.temp.path());
        assert!(
            broker.capture(ScanLimits::default()).is_err(),
            "captured replaced {node}"
        );
        assert!(
            broker.revalidate_capture(&stamp).is_err(),
            "revalidated replaced {node}"
        );
        assert!(
            broker.read_file(&path("note.md"), 64).is_err(),
            "read replaced {node}"
        );
        assert!(
            broker
                .validate_root_identities(approved.0, approved.1)
                .is_err()
        );
        assert_eq!(inventory(fixture.temp.path()), before);
    }
}

#[test]
fn replacement_after_actual_read_is_rejected_before_returning_content() {
    for single_page in [false, true] {
        for node in ["root", "marker", "data"] {
            let fixture = Fixture::new();
            let mut broker = fixture.initialize();
            fs::write(fixture.root.join("note.md"), b"fixture").unwrap();
            let target = match node {
                "root" => fixture.root.clone(),
                "marker" => fixture.root.join(".workbench/vault.json"),
                _ => fixture.data.clone(),
            };
            let moved = fixture.temp.path().join("moved");
            let observed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let ran = observed.clone();
            let _hook = read_hook(move || {
                fs::rename(&target, &moved).unwrap();
                if node == "marker" {
                    fs::copy(&moved, &target).unwrap();
                } else {
                    fs::create_dir(&target).unwrap();
                }
                ran.store(true, Ordering::Release);
            });
            let result = if single_page {
                broker.read_file(&path("note.md"), 64).map(|_| ())
            } else {
                broker.capture(ScanLimits::default()).map(|_| ())
            };
            assert!(observed.load(Ordering::Acquire));
            assert!(result.is_err(), "returned content after replacing {node}");
        }
    }
}
