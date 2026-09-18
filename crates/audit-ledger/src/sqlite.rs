//! 真实 SQLite Adapter：写事务串行分配 offset，提交后才返回成功，失败不泄漏底层路径。

mod inspection;
mod query;
mod schema;
mod segment;
mod storage;
mod upgrade;
mod verify;

// 私有测试共用同一份夹具模块；重复 `#[path]` 加载同一文件会被 clippy 判为重复模块。
#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod support_tests;

#[cfg(test)]
#[path = "../tests/support/configuration.rs"]
mod configuration_tests;

#[cfg(test)]
#[path = "../tests/support/planning.rs"]
mod planning_tests;

use crate::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord, AuditSegmentSummary,
    AuditVerificationCost, codec,
};
use jiff::Timestamp;
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use std::{path::Path, sync::Mutex, sync::MutexGuard, time::Duration};
use storage::{CreatePolicy, Storage};

/// 每段固定的审计记录条数；段边界只由 offset 决定，是磁盘格式的一部分。
pub const AUDIT_SEGMENT_RECORDS: u64 = segment::SEGMENT_RECORDS;

/// Unix 私有本地目录中的权威审计账本；不共享或暴露 SQLite 连接。
///
/// 多实例通过 BEGIN IMMEDIATE 串行追加。账本按 [`AUDIT_SEGMENT_RECORDS`] 条固定分段，
/// 每次 append/query 重算整条段链、走读当前打开段并核对 head，再只验证实际读到的段，
/// 因此成本是 O(段数 + 读到的段 × 段大小) 而不是 O(完整账本字节数)；
/// 全量验证由 [`SqliteAuditLedger::verify_all`] 提供。单条规范记录最多 1 MiB。
/// 目录须位于 Vault 外，父目录可信。
pub struct SqliteAuditLedger {
    connection: Mutex<Session>,
    storage: Storage,
}

/// 连接与「最近一次验证规模」一起上锁，保证读到的计数与该次操作对得上。
struct Session {
    connection: Connection,
    cost: AuditVerificationCost,
}

impl SqliteAuditLedger {
    /// 打开部署批准、预先存在的绝对私有目录，固定使用 audit.sqlite3。
    ///
    /// Unix 目录权限必须 0700，数据库与侧文件必须 0600、同所有者且无链接。
    /// 不支持非 Unix；不创建目录、不修复权限、不重置已有空库/损坏库或迁移未知格式。
    /// 只接受 v2 分段格式；v1 旧库返回 `UnsupportedSchema`，须先调用
    /// [`SqliteAuditLedger::upgrade_existing`]。
    /// 可信父目录和同 UID 进程不得恶意更名；本 Interface 不是对抗同权限管理员的隔离。
    pub fn open(private_directory: &Path) -> Result<Self, AuditError> {
        Self::open_mode(private_directory, CreatePolicy::AllowNew)
    }

    /// 只打开已有的 audit.sqlite3；缺失返回 MissingStore，不创建数据库或修复坏库。
    ///
    /// 目录、文件、侧文件与完整性要求同 open。合法既有库仍允许追加审计和创建 SQLite 侧文件，
    /// 本方法不是只读入口；存在性检查后文件消失也不会回退到初始化。
    pub fn open_existing(private_directory: &Path) -> Result<Self, AuditError> {
        Self::open_mode(private_directory, CreatePolicy::ExistingOnly)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Session>, AuditError> {
        self.connection.lock().map_err(|_| AuditError::Unavailable)
    }

    fn open_mode(private_directory: &Path, policy: CreatePolicy) -> Result<Self, AuditError> {
        let storage = Storage::prepare(private_directory, policy)?;
        let mut connection = connect(&storage)?;
        check_configuration(&connection)?;
        let cost = {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(unavailable)?;
            schema::initialize(&transaction, storage.created)?;
            let anchored = verify::anchor(&transaction)?;
            storage.check()?;
            transaction.commit().map_err(unavailable)?;
            anchored.cost
        };
        let mode: String = connection
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .map_err(unavailable)?;
        if mode != "wal" {
            return Err(AuditError::Unavailable);
        }
        check_configuration(&connection)?;
        storage.check()?;
        Ok(Self {
            connection: Mutex::new(Session { connection, cost }),
            storage,
        })
    }
}

impl AuditLedger for SqliteAuditLedger {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        // 在取得写锁前拒绝明显过大的事件；完整记录仍在提交前二次检查。
        event.payload().validate()?;
        if codec::canonical(&event)?.len() > codec::MAX_RECORD_BYTES {
            return Err(AuditError::RecordTooLarge);
        }
        self.storage.check()?;
        let mut session = self.lock()?;
        let transaction = session
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        schema::validate(&transaction, schema::VERSION)?;
        // 幂等键走唯一索引定位，再连带验证它所在的整段，绝不返回未验证的回执。
        let replay = query::replay_offset(
            &transaction,
            event.event_id().as_str(),
            codec::phase(event.phase()),
        )?;
        let anchored = verify::anchor(&transaction)?;
        let mut cost = anchored.cost;
        if let Some(offset) = replay {
            let (record, replayed) = verify::record_at(&transaction, &anchored, offset)?;
            cost.merge(replayed);
            if record.event() != &event {
                return Err(AuditError::IdempotencyConflict {
                    event_id: event.event_id().clone(),
                    phase: event.phase(),
                });
            }
            self.storage.check()?;
            transaction.commit().map_err(unavailable)?;
            session.cost = cost;
            return Ok(record);
        }
        let offset = anchored
            .last_offset
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or(AuditError::Unavailable)?;
        let record = codec::build(offset, Timestamp::now(), anchored.last_hash, event)?;
        let encoded = codec::persistable(&record)?;
        transaction
            .execute(
                "INSERT INTO audit_events(offset,event_id,phase,record_json) VALUES(?1,?2,?3,?4)",
                params![
                    offset as i64,
                    record.event().event_id().as_str(),
                    codec::phase(record.event().phase()),
                    encoded
                ],
            )
            .map_err(unavailable)?;
        // 写满一段就在同一事务里封存它；段摘要与最后一条记录要么一起可见，要么都不可见。
        let index = segment::index_of(offset)?;
        if offset == segment::last_offset(index)? {
            let previous_segment = anchored
                .summaries
                .last()
                .map_or_else(segment::genesis, AuditSegmentSummary::segment_hash);
            let first_previous = anchored.summaries.last().map_or_else(
                crate::IntegrityHash::genesis,
                AuditSegmentSummary::last_hash,
            );
            let summary = segment::summarize(
                index,
                first_previous,
                record.record_hash(),
                previous_segment,
            )?;
            segment::seal(&transaction, &summary)?;
        }
        let changed = transaction
            .execute(
                "UPDATE audit_meta SET last_offset=?1, last_hash=?2 WHERE id=1",
                params![offset as i64, record.record_hash().to_hex()],
            )
            .map_err(unavailable)?;
        if changed != 1 {
            return Err(AuditError::CorruptStore);
        }
        self.storage.check()?;
        transaction.commit().map_err(unavailable)?;
        self.storage.check()?;
        session.cost = cost;
        Ok(record)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.storage.check()?;
        let mut session = self.lock()?;
        let transaction = session
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(unavailable)?;
        schema::validate(&transaction, schema::VERSION)?;
        let anchored = verify::anchor(&transaction)?;
        let mut cost = anchored.cost;
        let (records, queried) = query::run(&transaction, &anchored, filter)?;
        cost.merge(queried);
        self.storage.check()?;
        transaction.commit().map_err(unavailable)?;
        session.cost = cost;
        Ok(records)
    }
}

fn connect(storage: &Storage) -> Result<Connection, AuditError> {
    // 两种模式均不让 SQLite 自建主文件；初始化只由 Storage 的明确创建策略负责。
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW
        | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE;
    let connection = Connection::open_with_flags(&storage.database, flags).map_err(unavailable)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(unavailable)?;
    connection
        .execute_batch(
            "PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;
            PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;",
        )
        .map_err(unavailable)?;
    Ok(connection)
}

fn unavailable(_error: rusqlite::Error) -> AuditError {
    AuditError::Unavailable
}

fn check_configuration(connection: &Connection) -> Result<(), AuditError> {
    // 未知 pragma 会被 SQLite 忽略；逐项核读而非把执行无错视为配置成功。
    for (query, expected) in [
        ("PRAGMA synchronous", 2),
        ("PRAGMA trusted_schema", 0),
        ("PRAGMA foreign_keys", 1),
        ("PRAGMA busy_timeout", 5000),
        ("PRAGMA fullfsync", 1),
        ("PRAGMA checkpoint_fullfsync", 1),
    ] {
        let actual: i64 = connection
            .query_row(query, [], |row| row.get(0))
            .map_err(unavailable)?;
        if actual != expected {
            return Err(AuditError::Unavailable);
        }
    }
    Ok(())
}
