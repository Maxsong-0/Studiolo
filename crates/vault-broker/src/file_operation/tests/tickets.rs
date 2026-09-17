//! 借用票据只延长 owner 的决定窗口，不增加执行次数或取消其他请求的能力。
use super::*;

#[test]
fn repeated_borrow_returns_original_receipt_without_touching_later_content() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    let ticket = broker
        .prepare_file_operation(delete(original, "repeat"), metadata())
        .unwrap();
    intent(&mut broker, &ticket);
    let receipt = broker.commit_file_operation(&ticket).unwrap();
    let state = mutation::load(&broker).unwrap();
    fs::write(fixture.root.join("note.md"), b"later independent content").unwrap();
    assert_eq!(broker.commit_file_operation(&ticket).unwrap(), receipt);
    assert_eq!(
        mutation::load(&broker).unwrap().next_sequence,
        state.next_sequence
    );
    assert!(broker.write_status().unwrap().unresolved);
    flush(&mut broker);
    assert_eq!(broker.commit_file_operation(&ticket).unwrap(), receipt);
    assert_eq!(
        fs::read(fixture.root.join("note.md")).unwrap(),
        b"later independent content"
    );
    assert_eq!(
        broker.write_status().unwrap().committed_through,
        receipt.sequence
    );
    assert_eq!(broker.list_tombstones(64, None).unwrap().entries.len(), 1);
    assert!(broker.pending_file_operation_audit(64).unwrap().is_empty());
}

#[test]
fn uncertain_writer_cannot_cancel_a_prepared_ticket_or_publish_an_outcome() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    let ticket = broker
        .prepare_file_operation(delete(original, "uncertain"), metadata())
        .unwrap();
    intent(&mut broker, &ticket);
    let before = super::super::digest(&mutation::load(&broker).unwrap()).unwrap();
    // 与 journal 持久化失败使用相同的实例毒化位，不伪造某个已成功的写入。
    broker.writer.poison();
    assert!(matches!(
        broker.cancel_file_operation(ticket),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(
        super::super::digest(&mutation::load(&broker).unwrap()).unwrap(),
        before
    );
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
    drop(broker);
    let broker = fixture.reopen();
    let pending = broker.file_operation_status().unwrap().unwrap();
    assert_eq!(pending.phase, FileOperationPhase::Prepared);
    assert!(pending.receipt.is_none());
    assert!(broker.pending_file_operation_audit(64).unwrap().is_empty());
}

#[test]
fn old_terminal_ticket_never_cancels_a_different_active_request() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    let request_a = delete(original, "a");
    let ticket_a = broker
        .prepare_file_operation(request_a.clone(), metadata())
        .unwrap();
    intent(&mut broker, &ticket_a);
    let receipt_a = broker.cancel_file_operation(ticket_a).unwrap();
    flush(&mut broker);
    let ticket_b = broker
        .prepare_file_operation(delete(original, "b"), metadata())
        .unwrap();
    let before = super::super::digest(&mutation::load(&broker).unwrap()).unwrap();
    let old_ticket = broker
        .prepare_file_operation(request_a, metadata())
        .unwrap();
    assert_eq!(broker.cancel_file_operation(old_ticket).unwrap(), receipt_a);
    assert_eq!(
        super::super::digest(&mutation::load(&broker).unwrap()).unwrap(),
        before
    );
    let pending = broker.file_operation_status().unwrap().unwrap();
    assert_eq!(pending.operation_id, ticket_b.preview().operation_id);
    assert_eq!(pending.phase, FileOperationPhase::Prepared);
    assert!(pending.receipt.is_none());
    let events = broker.pending_file_operation_audit(64).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].event().operation_id,
        ticket_b.preview().operation_id
    );
    assert_eq!(events[0].event().phase, ContentAuditPhase::Intent);
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
}
