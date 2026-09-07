//! agent 私有库 `agent.sqlite3`：类型摘要、实例登记与读取审计派生索引。
//!
//! 权威审计仍是 AuditLedger 账本；本库里的 `reads` 只是可从账本重建的查询索引。
//! 类型 token 明文只在签发时返回一次，库中只存 sha256。

use std::{path::Path, sync::Mutex, time::Duration};

use rusqlite::{Connection, OpenFlags, OptionalExtension as _, TransactionBehavior, params};
use sha2::{Digest as _, Sha256};

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedToken {
    /// 类型名。
    pub agent_type: String,
    /// 明文 token（`sag1_` + 64 位十六进制）。
    pub token: String,
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
    /// 打开（必要时初始化）部署批准的绝对私有目录下的 `agent.sqlite3`。
    pub fn open(private_directory: &Path) -> Result<Self, SupplyError> {
        Self::open_mode(private_directory, CreatePolicy::AllowNew)
    }

    /// 只打开已有库；缺失返回 `MissingStore`。
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
            transaction
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS agent_type (
                        name TEXT PRIMARY KEY CHECK (length(name) BETWEEN 1 AND 32),
                        token_sha256 TEXT NOT NULL UNIQUE CHECK (length(token_sha256) = 64),
                        issued_at TEXT NOT NULL,
                        revoked_at TEXT);
                     CREATE TABLE IF NOT EXISTS agent_instance (
                        id TEXT PRIMARY KEY,
                        type_name TEXT NOT NULL REFERENCES agent_type(name),
                        session_digest TEXT NOT NULL,
                        registered_at TEXT NOT NULL,
                        last_seen_at TEXT NOT NULL);
                     CREATE TABLE IF NOT EXISTS reads (
                        seq INTEGER PRIMARY KEY,
                        recorded_at_ms INTEGER NOT NULL,
                        recorded_at TEXT NOT NULL,
                        type_name TEXT NOT NULL,
                        instance_id TEXT NOT NULL,
                        path TEXT NOT NULL,
                        revision TEXT NOT NULL,
                        tool TEXT NOT NULL,
                        ledger_offset INTEGER NOT NULL UNIQUE);
                     CREATE INDEX IF NOT EXISTS reads_by_type ON reads(type_name, recorded_at_ms);
                     CREATE INDEX IF NOT EXISTS reads_by_instance ON reads(instance_id, recorded_at_ms);
                     CREATE INDEX IF NOT EXISTS reads_by_path ON reads(path, recorded_at_ms);
                     CREATE INDEX IF NOT EXISTS reads_by_time ON reads(recorded_at_ms);
                     CREATE TABLE IF NOT EXISTS outage (
                        id TEXT PRIMARY KEY,
                        started_at TEXT NOT NULL,
                        ended_at TEXT,
                        denied_count INTEGER NOT NULL DEFAULT 0,
                        reason TEXT NOT NULL);",
                )
                .map_err(unavailable)?;
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

    /// 签发（或轮换）一个类型的 token；旧 token 立即失效。明文只返回这一次。
    pub fn issue_type(&self, name: &str, now: &str) -> Result<IssuedToken, SupplyError> {
        validate_type_name(name)?;
        let token = format!("{TOKEN_PREFIX}{}", random_hex_64());
        let digest = sha256_hex(&token);
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO agent_type (name, token_sha256, issued_at, revoked_at)
                 VALUES (?1, ?2, ?3, NULL)
                 ON CONFLICT(name) DO UPDATE SET token_sha256 = excluded.token_sha256,
                     issued_at = excluded.issued_at, revoked_at = NULL",
                params![name, digest, now],
            )
            .map_err(unavailable)?;
        Ok(IssuedToken {
            agent_type: name.to_owned(),
            token,
        })
    }

    /// 吊销类型：即刻拒绝新握手与后续调用。返回是否存在该类型。
    pub fn revoke_type(&self, name: &str, now: &str) -> Result<bool, SupplyError> {
        validate_type_name(name)?;
        let connection = self.lock()?;
        let changed = connection
            .execute(
                "UPDATE agent_type SET revoked_at = ?2 WHERE name = ?1 AND revoked_at IS NULL",
                params![name, now],
            )
            .map_err(unavailable)?;
        Ok(changed == 1)
    }

    /// 已登记的类型及其状态（名字、签发时间、是否吊销）；不返回摘要。
    pub fn types(&self) -> Result<Vec<(String, String, bool)>, SupplyError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT name, issued_at, revoked_at IS NOT NULL FROM agent_type ORDER BY name")
            .map_err(unavailable)?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(unavailable)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
    }

    /// 用明文 token 认证：返回未吊销的类型名；格式不符、未知或已吊销都返回 `None`。
    pub fn authenticate(&self, token: &str) -> Result<Option<String>, SupplyError> {
        if !token.starts_with(TOKEN_PREFIX)
            || token.len() != TOKEN_PREFIX.len() + 64
            || !token[TOKEN_PREFIX.len()..]
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Ok(None);
        }
        let digest = sha256_hex(token);
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT name FROM agent_type WHERE token_sha256 = ?1 AND revoked_at IS NULL",
                params![digest],
                |row| row.get(0),
            )
            .optional()
            .map_err(unavailable)
    }

    /// 类型是否仍有效（用于每次 tool 调用的即刻吊销检查）。
    pub fn type_is_active(&self, name: &str) -> Result<bool, SupplyError> {
        let connection = self.lock()?;
        let active: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM agent_type WHERE name = ?1 AND revoked_at IS NULL",
                params![name],
                |row| row.get(0),
            )
            .optional()
            .map_err(unavailable)?;
        Ok(active.is_some())
    }

    /// 注册一个实例：ULID id，绑定类型与会话摘要；实例无独立权限。
    pub fn register_instance(
        &self,
        agent_type: &str,
        session_id: &str,
        now: &str,
    ) -> Result<Instance, SupplyError> {
        let id = ulid::Ulid::generate().to_string();
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO agent_instance (id, type_name, session_digest, registered_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![id, agent_type, sha256_hex(session_id), now],
            )
            .map_err(unavailable)?;
        Ok(Instance {
            id,
            agent_type: agent_type.to_owned(),
            registered_at: now.to_owned(),
        })
    }

    /// 更新实例最近活动时间。
    pub fn touch_instance(&self, instance_id: &str, now: &str) -> Result<(), SupplyError> {
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE agent_instance SET last_seen_at = ?2 WHERE id = ?1",
                params![instance_id, now],
            )
            .map_err(unavailable)?;
        Ok(())
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

impl AgentStore {
    /// 记录一次审计不可用窗口的开始（有界补记：同一 outage 只记一行）。
    pub fn record_outage_started(
        &self,
        outage_id: &str,
        now: &str,
        reason: &str,
    ) -> Result<(), SupplyError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT OR IGNORE INTO outage (id, started_at, ended_at, denied_count, reason)
                 VALUES (?1, ?2, NULL, 0, ?3)",
                params![outage_id, now, reason],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    /// 降级窗口内每次拒绝加一（有界：计数封顶于 i64）。
    pub fn record_outage_denial(&self, outage_id: &str) -> Result<(), SupplyError> {
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE outage SET denied_count = MIN(denied_count + 1, 9223372036854775807) WHERE id = ?1",
                params![outage_id],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    /// 标记 outage 结束。
    pub fn record_outage_ended(&self, outage_id: &str, now: &str) -> Result<(), SupplyError> {
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE outage SET ended_at = ?2 WHERE id = ?1 AND ended_at IS NULL",
                params![outage_id, now],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    /// 全部 outage 记录：(id, started_at, ended_at, denied_count, reason)。
    #[allow(clippy::type_complexity)]
    pub fn outages(
        &self,
    ) -> Result<Vec<(String, String, Option<String>, i64, String)>, SupplyError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, started_at, ended_at, denied_count, reason FROM outage ORDER BY started_at")
            .map_err(unavailable)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .map_err(unavailable)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
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

fn random_hex_64() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn unavailable(_: rusqlite::Error) -> SupplyError {
    SupplyError::StoreUnavailable
}
