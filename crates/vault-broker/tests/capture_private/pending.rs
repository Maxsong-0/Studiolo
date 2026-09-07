use super::*;

#[test]
fn every_unresolved_stage_blocks_capture_revalidation_and_page_read_without_finishing() {
    for point in [1, 2, 3, 4, 5] {
        let fixture = Fixture::new();
        let mut broker = fixture.initialize();
        let stamp = capture(&mut broker).stamp;
        interrupt(&broker, point);
        assert!(broker.mutate(request()).is_err());
        let before = inventory(fixture.temp.path());
        assert!(matches!(
            broker.capture(ScanLimits::default()),
            Err(BrokerError::MutationUnresolved)
        ));
        assert!(matches!(
            broker.revalidate_capture(&stamp),
            Err(BrokerError::MutationUnresolved)
        ));
        assert!(matches!(
            broker.read_file(&path("note.md"), 64),
            Err(BrokerError::MutationUnresolved)
        ));
        let status = broker.write_status().unwrap();
        assert!(status.unresolved);
        assert_eq!(status.committed_through, 0);
        assert_eq!(inventory(fixture.temp.path()), before);
        drop(broker);
        let mut reopened = fixture.open();
        if point >= 4 {
            // 确定 Completed 可由既有重开协议补水位；capture 自身不承担恢复。
            assert_eq!(capture(&mut reopened).stamp.committed_through(), 1);
            assert_eq!(
                reopened.read_file(&path("note.md"), 64).unwrap().content,
                b"intended"
            );
        } else {
            assert!(matches!(
                reopened.capture(ScanLimits::default()),
                Err(BrokerError::MutationUnresolved)
            ));
        }
    }
}

#[test]
fn legacy_unknown_and_in_process_uncertainty_do_not_produce_readable_captures() {
    for legacy in [false, true] {
        let fixture = Fixture::new();
        let mut broker = fixture.initialize();
        let stamp = capture(&mut broker).stamp;
        if legacy {
            let mut state = crate::mutation::load(&broker).unwrap();
            state.legacy_unresolved = true;
            fixture.restore_journal(&serde_json::to_vec(&state).unwrap());
        } else {
            broker.writer.poison();
        }
        let before = inventory(fixture.temp.path());
        assert!(matches!(
            broker.capture(ScanLimits::default()),
            Err(BrokerError::MutationUnresolved)
        ));
        assert!(matches!(
            broker.revalidate_capture(&stamp),
            Err(BrokerError::MutationUnresolved)
        ));
        assert!(matches!(
            broker.read_file(&path("note.md"), 64),
            Err(BrokerError::MutationUnresolved)
        ));
        assert_eq!(inventory(fixture.temp.path()), before);
    }
}
