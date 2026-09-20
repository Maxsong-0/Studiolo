//! agent 私有库 `agent.sqlite3`：类型摘要、实例登记与读取审计派生索引。
//!
//! 权威审计仍是 AuditLedger 账本；本库里的 `reads` 只是可从账本重建的查询索引。
//! 类型 token 明文只在签发时返回一次，库中只存 sha256。

use std::{path::Path, sync::Mutex, time::Duration};

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};

mod children;
mod identity;
mod outage;
mod schema;

use crate::{
    SupplyError,
    storage::{CreatePolicy, Storage},
};

const TOKEN_PREFIX: &str = "sag1_";

/// 一次读取审计的派生索引行；`recorded_at_ms` 为 Unix 毫秒，`recorded_at` 为 RFC 3339 显示值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadRecord {
    /// 账本记录时间（Unix 毫秒）。
    pub recorded_at_ms: i64,
    /// 账本记录时间（RFC 3339）。
    pub recorded_at: String,
    /// AI 类型。
    pub agent_type: String,
    /// 实例 id。
    pub instance_id: String,
    /// 逻辑路径。
    pub path: String,
    /// 供给时的 revision（十六进制）。
    pub revision: String,
    /// 触发读取的 tool。
    pub tool: String,
    /// 账本偏移，用于回查权威记录。
    pub ledger_offset: u64,
}

/// 读取审计查询：类型 × 实例 × 时间 × 路径前缀。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadQuery {
    /// 按类型过滤。
    pub agent_type: Option<String>,
    /// 按实例过滤。
    pub instance_id: Option<String>,
    /// 起始（含，Unix 毫秒）。
    pub since_ms: Option<i64>,
    /// 截止（含，Unix 毫秒）。
    pub until_ms: Option<i64>,
    /// 逻辑路径前缀。
    pub path_prefix: Option<String>,
    /// 最多返回条数（1..=10000）。
    pub limit: usize,
}

/// 类型签发结果：明文只出现在这里一次。
///
/// 明文只能由调用方经受控通道交付一次（当前唯一出口是 `studiolo-agent` 的控制终端门禁）。
/// 因此本类型**不派生** `Debug`：派生实现会让任何 `{:?}`、日志或 panic 消息复述明文，
/// 那是与「只显示一次」等价的泄漏面。
#[derive(Clone, PartialEq, Eq)]
pub struct IssuedToken {
    /// 类型名。
    pub agent_type: String,
    /// 明文 token（`sag1_` + 64 位十六进制）。
    pub token: String,
}

impl std::fmt::Debug for IssuedToken {
    /// 只报类型名与明文长度；绝不复述明文本身。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssuedToken")
            .field("agent_type", &self.agent_type)
            .field(
                "token",
                &format_args!("<{} 字节明文，已隐去>", self.token.len()),
            )
            .finish()
    }
}

/// 已注册的实例。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    /// 实例 id（ULID）。
    pub id: String,
    /// 所属类型。
    pub agent_type: String,
    /// 注册时间（RFC 3339）。
    pub registered_at: String,
}

/// agent 私有库句柄；不共享连接。
pub struct AgentStore {
    connection: Mutex<Connection>,
    storage: Storage,
}

impl AgentStore {
    /// 打开部署批准的绝对私有目录；仅初始化本次独占新建的 `agent.sqlite3`。
    pub fn open(private_directory: &Path) -> Result<Self, SupplyError> {
        Self::open_mode(private_directory, CreatePolicy::AllowNew)
    }

    /// 只打开并验证已有库；缺失返回 `MissingStore`，不建表、不迁移或修补损坏库。
    pub fn open_existing(private_directory: &Path) -> Result<Self, SupplyError> {
        Self::open_mode(private_directory, CreatePolicy::ExistingOnly)
    }

    fn open_mode(private_directory: &Path, policy: CreatePolicy) -> Result<Self, SupplyError> {
        let storage = Storage::prepare(private_directory, policy)?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE;
        let mut connection =
            Connection::open_with_flags(&storage.database, flags).map_err(unavailable)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(unavailable)?;
        connection
            .execute_batch(
                "PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;",
            )
            .map_err(unavailable)?;
        {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(unavailable)?;
            // 既有库先验格式，未知/缺损结构不能经过 journal_mode 设置或补表变成有效。
            schema::initialize(&transaction, storage.created)?;
            storage.check()?;
            transaction.commit().map_err(unavailable)?;
        }
        let mode: String = connection
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .map_err(unavailable)?;
        if mode != "wal" {
            return Err(SupplyError::StoreUnavailable);
        }
        storage.check()?;
        Ok(Self {
            connection: Mutex::new(connection),
            storage,
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, SupplyError> {
        self.storage.check()?;
        self.connection
            .lock()
            .map_err(|_| SupplyError::StoreUnavailable)
    }

    /// 账本追加成功后写入派生索引；同一账本偏移只记一次。
    pub fn record_read(&self, record: &ReadRecord) -> Result<(), SupplyError> {
        let connection = self.lock()?;
        insert_read(&connection, record)
    }

    /// 四维查询，按时间升序。
    pub fn query_reads(&self, query: &ReadQuery) -> Result<Vec<ReadRecord>, SupplyError> {
        let limit = query.limit.clamp(1, 10_000);
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT recorded_at_ms, recorded_at, type_name, instance_id, path, revision, tool, ledger_offset
                 FROM reads
                 WHERE (?1 IS NULL OR type_name = ?1)
                   AND (?2 IS NULL OR instance_id = ?2)
                   AND (?3 IS NULL OR recorded_at_ms >= ?3)
                   AND (?4 IS NULL OR recorded_at_ms <= ?4)
                   AND (?5 IS NULL OR substr(path, 1, length(?5)) = ?5)
                 ORDER BY recorded_at_ms, seq
                 LIMIT ?6",
            )
            .map_err(unavailable)?;
        let rows = statement
            .query_map(
                params![
                    query.agent_type,
                    query.instance_id,
                    query.since_ms,
                    query.until_ms,
                    query.path_prefix,
                    limit as i64
                ],
                |row| {
                    Ok(ReadRecord {
                        recorded_at_ms: row.get(0)?,
                        recorded_at: row.get(1)?,
                        agent_type: row.get(2)?,
                        instance_id: row.get(3)?,
                        path: row.get(4)?,
                        revision: row.get(5)?,
                        tool: row.get(6)?,
                        ledger_offset: row.get::<_, i64>(7)?.max(0) as u64,
                    })
                },
            )
            .map_err(unavailable)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
    }

    /// 从账本重放重建整个派生索引；单事务，失败不留半截。
    pub fn rebuild_reads<I: IntoIterator<Item = ReadRecord>>(
        &self,
        records: I,
    ) -> Result<usize, SupplyError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        transaction
            .execute("DELETE FROM reads", [])
            .map_err(unavailable)?;
        let mut count = 0;
        for record in records {
            insert_read(&transaction, &record)?;
            count += 1;
        }
        transaction.commit().map_err(unavailable)?;
        Ok(count)
    }
}

fn insert_read(connection: &Connection, record: &ReadRecord) -> Result<(), SupplyError> {
    connection
        .execute(
            "INSERT OR IGNORE INTO reads
                (recorded_at_ms, recorded_at, type_name, instance_id, path, revision, tool, ledger_offset)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                record.recorded_at_ms,
                record.recorded_at,
                record.agent_type,
                record.instance_id,
                record.path,
                record.revision,
                record.tool,
                record.ledger_offset as i64
            ],
        )
        .map_err(unavailable)?;
    Ok(())
}

/// 类型名只允许小写字母、数字与连字符，1..=32 字符。
pub fn validate_type_name(name: &str) -> Result<(), SupplyError> {
    let valid = (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        && !name.starts_with('-');
    if valid {
        Ok(())
    } else {
        Err(SupplyError::InvalidTypeName)
    }
}

fn unavailable(_: rusqlite::Error) -> SupplyError {
    SupplyError::StoreUnavailable
}
