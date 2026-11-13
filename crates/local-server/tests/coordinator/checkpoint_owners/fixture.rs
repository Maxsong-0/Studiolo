//! 总窗口测试的真实 owner 装配与固定私有根。
use super::*;

pub(super) struct Setup {
    pub(super) fixture: Fixture,
    pub(super) workbench: Arc<crate::CanonicalWorkbench>,
    pub(super) human: Arc<SqliteAuditLedger>,
    pub(super) agent: SqliteAuditLedger,
    pub(super) owners: CheckpointOwners,
    pub(super) key: JobKey,
}
impl Setup {
    pub(super) async fn new(intent: bool) -> Self {
        let fixture = Fixture::new();
        let (mut broker, human) = source(&fixture);
        flush_content(&mut broker, &human, 0);
        let workbench = start(&fixture, broker, human.clone()).await;
        let config = Config::load(&fixture.config).unwrap();
        let sessions = Arc::new(
            SessionManager::open(
                &fixture.root.join("auth"),
                config.sessions().unwrap(),
                human.clone(),
                Arc::new(SystemSessionClock),
                Arc::new(OsSessionRandom),
            )
            .unwrap(),
        );
        let policy_path = target(&fixture, "policy");
        let policy = Arc::new(PolicyPlane::new(
            PolicyStore::open(&policy_path).unwrap(),
            human.clone(),
        ));
        let agent_path = target(&fixture, "agent");
        let agent_store = AgentStore::open(&agent_path).unwrap();
        agent_store
            .issue_type("codex", "2026-11-13T20:00:00Z")
            .unwrap();
        let audit_path = agent_path.join("audit");
        fs::create_dir(&audit_path).unwrap();
        fs::set_permissions(&audit_path, fs::Permissions::from_mode(0o700)).unwrap();
        let agent = SqliteAuditLedger::open(&audit_path).unwrap();
        let control = target(&fixture, "control");
        let board = BoardPlane::open(
            workbench.clone(),
            &control,
            &agent_path,
            &policy_path,
            &[],
            human.clone(),
        )
        .unwrap();
        let agents = board.checkpoint_agents().unwrap();
        let owners = CheckpointOwners::new(CheckpointInputs {
            workbench: workbench.clone(),
            sessions,
            audit: human.clone(),
            board: Some(board),
            bridge: None,
            agents: Some(agents),
            policy: Some(policy),
        })
        .unwrap();
        let key = JobKey {
            sequence: NonZeroU64::new(1).unwrap(),
            checkpoint_id: CheckpointId::parse("actual-owners").unwrap(),
        };
        if intent {
            human
                .append(backup_intent(&key, BackupTargetKind::LocalTemporaryRepository).unwrap())
                .unwrap();
        }
        Self {
            fixture,
            workbench,
            human,
            agent,
            owners,
            key,
        }
    }
    pub(super) fn request(&self, millis: u32) -> FreezeRequest {
        FreezeRequest {
            checkpoint_id: self.key.checkpoint_id.clone(),
            budget: FreezeBudget::new(millis).unwrap(),
        }
    }
    pub(super) fn targets(&self) -> OwnerExportPaths {
        let paths = OwnerExportPaths::under(&target(&self.fixture, "exports"));
        for path in paths.directories() {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        paths
    }
    pub(super) fn writable(&self, expected: bool) {
        for relative in [
            "auth/auth.sqlite3",
            "control/control.sqlite3",
            "agent/agent.sqlite3",
            "policy/policy.sqlite3",
            "audit/audit.sqlite3",
            "agent/audit/audit.sqlite3",
        ] {
            let db = rusqlite::Connection::open(self.fixture.root.join(relative)).unwrap();
            db.busy_timeout(Duration::ZERO).unwrap();
            let result = db.execute_batch("BEGIN IMMEDIATE");
            assert_eq!(result.is_ok(), expected, "{relative}");
            if result.is_ok() {
                db.execute_batch("ROLLBACK").unwrap();
            }
        }
    }
}
