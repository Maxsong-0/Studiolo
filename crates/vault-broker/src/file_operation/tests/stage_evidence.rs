//! 恢复候选必须先保全；解除未知槽后的常规 stage 清理不能销毁唯一证据。
use super::super::maintenance_review::Snapshot;
use super::*;

struct Scenario {
    fixture: Fixture,
    broker: FsVaultBroker,
    request: FileOperationRequest,
    stage: PathBuf,
    original: Revision,
    tombstone: String,
}
impl Scenario {
    fn new() -> Self {
        let fixture = Fixture::new();
        let (mut broker, original) = fixture.seed(b"old");
        let deleted = committed(&mut broker, delete(original, "delete-stage-source"));
        let tombstone = deleted.tombstone_id.unwrap();
        let request = restore(&tombstone, "restore-stage-evidence");
        let ticket = broker
            .prepare_file_operation(request.clone(), metadata())
            .unwrap();
        intent(&mut broker, &ticket);
        fault(&broker, 52);
        assert!(matches!(
            broker.commit_file_operation(&ticket),
            Err(BrokerError::MutationUnresolved)
        ));
        let active = mutation::load(&broker).unwrap().file_operation.unwrap();
        assert_eq!(active.phase, FileOperationPhase::InstallIssued);
        let stage = fixture
            .root
            .join(".workbench/staging")
            .join(active.stage.unwrap().name);
        Self {
            fixture,
            broker,
            request,
            stage,
            original,
            tombstone,
        }
    }
    fn reopen(self) -> Self {
        // 原实例票据已经丢失，仅通过重开的维护审阅继续。
        let Self {
            fixture,
            broker,
            request,
            stage,
            original,
            tombstone,
        } = self;
        drop(broker);
        let broker = fixture.reopen();
        Self {
            fixture,
            broker,
            request,
            stage,
            original,
            tombstone,
        }
    }
    fn preserve(&mut self) -> String {
        let review = self
            .broker
            .maintenance_inspect(64)
            .unwrap()
            .targets
            .remove(0);
        let ticket = self
            .broker
            .prepare_maintenance(
                &review.review_id,
                MaintenanceDecision::PreserveInstalling,
                MaintenanceActor::current().unwrap(),
            )
            .unwrap();
        self.broker
            .confirm_maintenance_intent(&ticket, 3, &"b".repeat(64))
            .unwrap();
        self.broker.commit_maintenance(ticket).unwrap();
        flush(&mut self.broker);
        for candidate in self.broker.pending_maintenance_audit(64).unwrap() {
            self.broker
                .confirm_maintenance_audit(candidate, 4, &"b".repeat(64))
                .unwrap();
        }
        let result = self
            .broker
            .file_operation_replay(&self.request, &metadata())
            .unwrap()
            .unwrap()
            .receipt
            .unwrap();
        assert_eq!(result.outcome, FileOperationOutcome::Indeterminate);
        assert_eq!(self.broker.write_status().unwrap().committed_through, 1);
        assert!(!self.broker.write_status().unwrap().unresolved);
        result.evidence_id.unwrap()
    }
    fn snapshot(&self, id: &str, role: &str) -> PathBuf {
        self.fixture
            .data
            .join("snapshots")
            .join(format!("file-operation-{id}-{role}.bin"))
    }
}

#[test]
fn changed_restore_stage_is_copied_before_releasing_it_to_normal_pruning() {
    let mut s = Scenario::new();
    let old = s.broker.maintenance_inspect(64).unwrap().targets.remove(0);
    fs::write(&s.stage, b"foreign-stage").unwrap();
    assert!(matches!(
        s.broker.prepare_maintenance(
            &old.review_id,
            MaintenanceDecision::PreserveInstalling,
            MaintenanceActor::current().unwrap()
        ),
        Err(BrokerError::CaptureChanged)
    ));
    assert!(s.broker.maintenance_pending().unwrap().is_none());
    s = s.reopen();
    let review = super::super::maintenance_review::capture(&s.broker)
        .unwrap()
        .unwrap();
    assert!(matches!(review.stage, Snapshot::Present { .. }));
    assert!(matches!(review.quarantine, Snapshot::NotObserved));
    let id = s.preserve();
    assert_eq!(
        fs::read(s.snapshot(&id, "stage")).unwrap(),
        b"foreign-stage"
    );
    assert_eq!(fs::read(s.snapshot(&id, "history")).unwrap(), b"old");
    assert!(!s.snapshot(&id, "quarantine").exists());
    assert_eq!(s.broker.prune_stages().unwrap().removed, 1);
    assert!(!s.stage.exists());
    assert_eq!(
        fs::read(s.snapshot(&id, "stage")).unwrap(),
        b"foreign-stage"
    );
    assert!(super::super::usage::actual(&s.broker).unwrap().0 >= b"foreign-stage".len() as u64);
}

#[test]
fn stage_as_the_only_remaining_copy_survives_preservation_and_later_pruning() {
    let mut s = Scenario::new();
    fs::remove_file(s.fixture.data.join("history").join(s.original.to_string())).unwrap();
    fs::remove_file(
        s.fixture
            .root
            .join(".workbench/deleted-objects")
            .join(format!("{}.object", s.tombstone)),
    )
    .unwrap();
    s = s.reopen();
    let review = super::super::maintenance_review::capture(&s.broker)
        .unwrap()
        .unwrap();
    assert!(matches!(review.original, Snapshot::Missing));
    let id = s.preserve();
    assert_eq!(s.broker.prune_stages().unwrap().removed, 1);
    assert_eq!(fs::read(s.snapshot(&id, "stage")).unwrap(), b"old");
    assert!(!s.snapshot(&id, "history").exists());
    assert!(!s.fixture.root.join("note.md").exists());
}

#[test]
fn missing_restore_stage_is_explicit_evidence_without_inventing_a_success() {
    let mut s = Scenario::new();
    fs::remove_file(&s.stage).unwrap();
    s = s.reopen();
    let review = super::super::maintenance_review::capture(&s.broker)
        .unwrap()
        .unwrap();
    assert!(matches!(review.stage, Snapshot::Missing));
    assert!(
        !super::super::maintenance_review::target(&review)
            .unwrap()
            .proposed_available
    );
    let id = s.preserve();
    assert!(!s.snapshot(&id, "stage").exists());
    assert_eq!(fs::read(s.snapshot(&id, "history")).unwrap(), b"old");
    assert_eq!(s.broker.prune_stages().unwrap().removed, 0);
    assert!(!s.fixture.root.join("note.md").exists());
}

#[test]
fn unobservable_restore_stage_keeps_the_original_slot_and_cannot_be_pruned() {
    let mut s = Scenario::new();
    fs::remove_file(&s.stage).unwrap();
    std::os::unix::fs::symlink("/unreadable-outside", &s.stage).unwrap();
    s = s.reopen();
    let review = super::super::maintenance_review::capture(&s.broker)
        .unwrap()
        .unwrap();
    assert!(matches!(review.stage, Snapshot::Unobservable { .. }));
    let target = super::super::maintenance_review::target(&review).unwrap();
    assert!(matches!(
        s.broker.prepare_maintenance(
            &target.review_id,
            MaintenanceDecision::PreserveInstalling,
            MaintenanceActor::current().unwrap()
        ),
        Err(BrokerError::MutationUnresolved)
    ));
    let pending = s.broker.file_operation_status().unwrap().unwrap();
    assert_eq!(pending.phase, FileOperationPhase::InstallIssued);
    assert!(pending.receipt.is_none());
    assert!(s.broker.maintenance_pending().unwrap().is_none());
    assert!(s.broker.prune_stages().is_err());
    assert!(
        fs::symlink_metadata(&s.stage)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_dir(s.fixture.data.join("snapshots"))
            .unwrap()
            .count(),
        0
    );
}
