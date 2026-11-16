use super::*;

pub(super) struct World {
    pub(super) ledger: PickyLedger,
    pub(super) store: MemoryStore,
    pub(super) clock: TestClock,
    pub(super) _fixture: Fixture,
    pub(super) seal: SealedCheckpoint,
    pub(super) finalizer: StubFinal,
}

impl World {
    pub(super) fn new() -> Self {
        let fixture = Fixture::new();
        let mut inputs = fixture.inputs();
        inputs.content_cutoff = InstantMillis::from_millis(1000);
        let seal = fixture
            .area("cp-0001")
            .seal(&SourceCatalog::new([]).unwrap(), inputs)
            .unwrap();
        Self {
            ledger: PickyLedger::new(),
            store: MemoryStore::default(),
            clock: TestClock::new(3_000),
            _fixture: fixture,
            seal,
            finalizer: StubFinal::default(),
        }
    }

    pub(super) fn run(
        &self,
        worker: &BackupWorker<'_>,
        key: &studiolo_backup::worker::state::JobKey,
        runner: &StubRunner,
        probe: &StubProbe,
    ) -> Result<JobOutcome, WorkerError> {
        worker.run(
            key,
            &self.seal,
            DataRemote { runner, probe },
            &self.finalizer,
        )
    }

    pub(super) fn worker<'a>(&'a self, target: BackupTargetKind) -> BackupWorker<'a> {
        BackupWorker::new(
            &self.ledger,
            &self.store,
            repository(),
            CredentialHolder::BackupWorker,
            target,
        )
        .expect("worker")
    }
}
