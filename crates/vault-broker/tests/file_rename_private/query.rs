//! 能力查询只读已有证据，不代替未决审计闭合或启用许可。
use super::*;

#[test]
fn query_is_read_only_before_enable_and_during_prepared_and_outcome_pending() {
    let fixture = Fixture::new();
    let (mut broker, revision) = fixture.seed(b"source");
    let before = fs::read(fixture.data.join("writer-state.json")).unwrap();
    assert_eq!(broker.file_operation_limits().unwrap(), Some(limits()));
    assert!(!broker.file_rename_enabled().unwrap());
    assert_eq!(
        fs::read(fixture.data.join("writer-state.json")).unwrap(),
        before
    );
    broker.enable_file_rename().unwrap();
    let r = request("note.md", "target.md", revision, "query-pending");
    let ticket = broker
        .prepare_file_operation(r.clone(), metadata())
        .unwrap();
    let prepared = fs::read(fixture.data.join("writer-state.json")).unwrap();
    assert_eq!(broker.file_operation_limits().unwrap(), Some(limits()));
    assert!(broker.file_rename_enabled().unwrap());
    assert_eq!(
        fs::read(fixture.data.join("writer-state.json")).unwrap(),
        prepared
    );
    // 已有在途文件操作时不能改写启用标记。
    assert!(matches!(
        broker.enable_file_rename(),
        Err(BrokerError::MutationUnresolved)
    ));
    intent(&mut broker, &ticket);
    let committed = broker.commit_file_operation(&ticket).unwrap();
    drop(ticket);
    let pending = broker
        .file_operation_replay(&r, &metadata())
        .unwrap()
        .unwrap();
    assert!(pending.audit_pending);
    let awaiting = fs::read(fixture.data.join("writer-state.json")).unwrap();
    assert_eq!(broker.file_operation_limits().unwrap(), Some(limits()));
    assert!(broker.file_rename_enabled().unwrap());
    assert_eq!(
        fs::read(fixture.data.join("writer-state.json")).unwrap(),
        awaiting
    );
    assert!(matches!(
        broker.enable_file_rename(),
        Err(BrokerError::MutationUnresolved)
    ));
    drop(broker);
    let mut broker = fixture.reopen();
    assert!(broker.file_rename_enabled().unwrap());
    assert!(
        broker
            .file_operation_replay(&r, &metadata())
            .unwrap()
            .unwrap()
            .audit_pending
    );
    flush(&mut broker);
    assert_eq!(receipt(&broker, &r), committed);
    assert!(
        !broker
            .file_operation_replay(&r, &metadata())
            .unwrap()
            .unwrap()
            .audit_pending
    );
    assert_eq!(fs::read(fixture.root.join("target.md")).unwrap(), b"source");
    assert!(!fixture.root.join("note.md").exists());
    assert_eq!(broker.write_status().unwrap().committed_through, 1);
}

#[test]
fn capability_queries_on_uninitialized_store_do_not_grant_or_create_it() {
    let fixture = Fixture::new();
    let broker = fixture.open();
    let before = fs::read(fixture.data.join("writer-state.json")).unwrap();
    let data_entries = fs::read_dir(&fixture.data).unwrap().count();
    let vault_entries = fs::read_dir(&fixture.root).unwrap().count();
    assert_eq!(broker.file_operation_limits().unwrap(), None);
    assert!(!broker.file_rename_enabled().unwrap());
    assert_eq!(
        fs::read(fixture.data.join("writer-state.json")).unwrap(),
        before
    );
    assert_eq!(fs::read_dir(&fixture.data).unwrap().count(), data_entries);
    assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), vault_entries);
}
