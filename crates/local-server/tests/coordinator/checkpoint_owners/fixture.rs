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
/// 真实装配、还没固定成一次作业的原实例；生产者测试直接拿它当启动层保留的那一组。
pub(super) struct Assembled {
    pub(super) fixture: Fixture,
    pub(super) workbench: Arc<crate::CanonicalWorkbench>,
    pub(super) human: Arc<SqliteAuditLedger>,
    pub(super) agent: SqliteAuditLedger,
    pub(super) inputs: CheckpointInputs,
}
impl Assembled {
    pub(super) async fn new() -> Self {
        Self::build(false, Arc::new(SystemSessionClock), 0).await
    }
    /// 会话时钟由用例拨：演练「会话过了空闲期」用。
    pub(super) async fn with_clock(clock: Arc<dyn SessionClock>) -> Self {
        Self::build(false, clock, 0).await
    }
    /// 写者那条正文的回执和账本对不上：1 是账本里同一个事件号换了动作，2 是回执记的哈希不对。
    pub(super) async fn with_bad_receipt(corrupt: u8) -> Self {
        Self::build(false, Arc::new(SystemSessionClock), corrupt).await
    }
    /// 主人登记过 TOTP（Auth 建在 v4）、登记过一个接入方实例：新根恢复的用例用它。撤权要挂上主人登记过的
    /// 因子，身份库那一步要有实例可吊销。
    pub(super) async fn with_owner_factor() -> Self {
        Self::build(true, Arc::new(SystemSessionClock), 0).await
    }
    async fn build(owner_factor: bool, clock: Arc<dyn SessionClock>, corrupt: u8) -> Self {
        let fixture = Fixture::new();
        let (mut broker, human) = source(&fixture);
        flush_content(&mut broker, &human, corrupt);
        let workbench = start(&fixture, broker, human.clone()).await;
        let config = Config::load(&fixture.config).unwrap();
        let auth = fixture.root.join("auth");
        let sessions = Arc::new(
            if owner_factor {
                // 撤权只认 v4（恢复码存储）：新目录经 v3→v4 的迁移入口一次建到 v4。
                SessionManager::migrate_v3(
                    &auth,
                    config.sessions().unwrap(),
                    human.clone(),
                    clock,
                    Arc::new(OsSessionRandom),
                )
            } else {
                SessionManager::open(
                    &auth,
                    config.sessions().unwrap(),
                    human.clone(),
                    clock,
                    Arc::new(OsSessionRandom),
                )
            }
            .unwrap(),
        );
        if owner_factor {
            super::restore::enroll(&sessions);
        }
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
        if owner_factor {
            agent_store
                .register_instance("codex", "restore-drill-session", "2026-11-13T20:00:00Z")
                .unwrap();
        }
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
        let inputs = CheckpointInputs {
            workbench: workbench.clone(),
            sessions,
            factors: None,
            audit: human.clone(),
            board: Some(board),
            bridge: None,
            policy: Some(policy),
        };
        Self {
            fixture,
            workbench,
            human,
            agent,
            inputs,
        }
    }
    /// 六个库现在能不能写：冻结期间都不能，放开之后都能。
    pub(super) fn writable(&self, expected: bool) {
        writable(&self.fixture, expected);
    }
}
impl Setup {
    pub(super) async fn new(intent: bool) -> Self {
        Self::of(Assembled::new().await, intent)
    }
    pub(super) fn of(assembled: Assembled, intent: bool) -> Self {
        let Assembled {
            fixture,
            workbench,
            human,
            agent,
            inputs,
        } = assembled;
        let owners = CheckpointOwners::new(inputs).unwrap();
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
        writable(&self.fixture, expected);
    }
}
fn writable(fixture: &Fixture, expected: bool) {
    for relative in [
        "auth/auth.sqlite3",
        "control/control.sqlite3",
        "agent/agent.sqlite3",
        "policy/policy.sqlite3",
        "audit/audit.sqlite3",
        "agent/audit/audit.sqlite3",
    ] {
        let db = rusqlite::Connection::open(fixture.root.join(relative)).unwrap();
        db.busy_timeout(Duration::ZERO).unwrap();
        let result = db.execute_batch("BEGIN IMMEDIATE");
        assert_eq!(result.is_ok(), expected, "{relative}");
        if result.is_ok() {
            db.execute_batch("ROLLBACK").unwrap();
        }
    }
}
