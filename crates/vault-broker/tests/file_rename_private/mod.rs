//! 真实临时目录故障契约；ACK值模拟已核验的owner边界，实际SQLite映射由local-server独立覆盖。
use super::*;
use crate::file_operation as ops;
mod budgets;
mod crashes;
mod large;
mod lifecycle;
mod preservation;
mod query;
mod races;
mod validation;
fn logical(s: &str) -> VaultPath {
    VaultPath::parse(s).unwrap()
}
fn request(from: &str, to: &str, revision: Revision, key: &str) -> FileOperationRequest {
    FileOperationRequest::Rename {
        from: logical(from),
        expected: revision,
        to: logical(to),
        idempotency_key: key.into(),
    }
}
fn setup(body: &[u8]) -> (Fixture, FsVaultBroker, Revision) {
    let fixture = Fixture::new();
    let (mut broker, revision) = fixture.seed(body);
    broker.enable_file_rename().unwrap();
    (fixture, broker, revision)
}
fn pending(broker: &mut FsVaultBroker, request: FileOperationRequest, point: u8) {
    let ticket = broker.prepare_file_operation(request, metadata()).unwrap();
    assert_eq!(ticket.intent().schema_version, 2);
    intent(broker, &ticket);
    fault(broker, point);
    assert!(matches!(
        broker.commit_file_operation(&ticket),
        Err(BrokerError::MutationUnresolved)
    ));
}
fn reconcile(broker: &mut FsVaultBroker, decision: MaintenanceDecision) -> MaintenanceTarget {
    flush(broker);
    let observed = broker.maintenance_inspect(1).unwrap().targets.remove(0);
    let ticket = broker
        .prepare_maintenance(
            &observed.review_id,
            decision,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    assert_eq!(ticket.intent().schema_version, Some(3));
    assert_eq!(ticket.intent().target, observed);
    broker
        .confirm_maintenance_intent(&ticket, 3, &"b".repeat(64))
        .unwrap();
    let receipt = broker.commit_maintenance(ticket).unwrap();
    assert_eq!(receipt.event.target, observed);
    let candidate = broker.pending_maintenance_audit(2).unwrap().remove(0);
    assert!(matches!(
        broker.confirm_maintenance_audit(candidate, 4, &"b".repeat(64)),
        Err(BrokerError::MutationUnresolved)
    ));
    flush(broker);
    for candidate in broker.pending_maintenance_audit(2).unwrap() {
        broker
            .confirm_maintenance_audit(candidate, 4, &"b".repeat(64))
            .unwrap();
    }
    assert!(!broker.write_status().unwrap().unresolved);
    observed
}
fn receipt(broker: &FsVaultBroker, r: &FileOperationRequest) -> FileOperationReceipt {
    broker
        .file_operation_replay(r, &metadata())
        .unwrap()
        .unwrap()
        .receipt
        .unwrap()
}
