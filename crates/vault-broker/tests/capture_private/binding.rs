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
        // canonical 根被换报根已变；控制目录、marker、data 与 history 被换报配对不符。
        let replaced = |error: &BrokerError| match node {
            "root" => matches!(error, BrokerError::RootChanged),
            _ => matches!(error, BrokerError::WriterBindingMismatch),
        };
        let captured = broker.capture(ScanLimits::default()).map(|_| ());
        assert!(
            matches!(&captured, Err(error) if replaced(error)),
            "captured replaced {node}: {captured:?}"
        );
        let revalidated = broker.revalidate_capture(&stamp);
        assert!(
            matches!(&revalidated, Err(error) if replaced(error)),
            "revalidated replaced {node}: {revalidated:?}"
        );
        let read = broker.read_file(&path("note.md"), 64).map(|_| ());
        assert!(
            matches!(&read, Err(error) if replaced(error)),
            "read replaced {node}: {read:?}"
        );
        let validated = broker.validate_root_identities(approved.0, approved.1);
        assert!(
            matches!(&validated, Err(error) if replaced(error)),
            "validated replaced {node}: {validated:?}"
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
            let replaced = match node {
                "root" => matches!(result, Err(BrokerError::RootChanged)),
                _ => matches!(result, Err(BrokerError::WriterBindingMismatch)),
            };
            assert!(
                replaced,
                "returned content after replacing {node}: {result:?}"
            );
        }
    }
}
