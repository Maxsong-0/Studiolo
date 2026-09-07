//! 验证预算共用固定窗口算法；封闭域只选择固定表和哈希域，不接受任意 SQL。

use crate::session::{SessionError, database::unavailable, secrets};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::net::IpAddr;

const WINDOW: i64 = 900_000;
const LIMIT: i64 = 5;
const NETWORK_BUCKETS: i64 = 64;

#[derive(Clone, Copy)]
pub(super) enum Scope {
    Totp,
    Recovery,
}

impl Scope {
    fn table(self) -> &'static str {
        match self {
            Self::Totp => "totp_rates",
            Self::Recovery => "recovery_rates",
        }
    }

    fn domain(self) -> &'static [u8] {
        match self {
            Self::Totp => b"studiolo-totp-rate-ip-v1\0",
            Self::Recovery => b"studiolo-recovery-rate-ip-v1\0",
        }
    }
}

pub(super) fn reserve(
    db: &Connection,
    scope: Scope,
    now: i64,
    source: Option<IpAddr>,
) -> Result<Result<(), SessionError>, SessionError> {
    if db.is_autocommit() {
        return Err(SessionError::StorageUnavailable);
    }
    let floor: i64 = db
        .query_row("SELECT clock_floor FROM auth_meta WHERE id=1", [], |r| {
            r.get(0)
        })
        .map_err(unavailable)?;
    if now < 0 || now < floor {
        return Ok(Err(SessionError::ClockRollback));
    }
    if now != floor {
        return Err(SessionError::CorruptStore);
    }
    let table = scope.table();
    // 只回收完整固定窗口已经结束的网络桶，不驱逐任何仍生效的限制。
    db.execute(
        &format!("DELETE FROM {table} WHERE bucket LIKE 'ip:%' AND ?1-window_started>=?2"),
        params![now, WINDOW],
    )
    .map_err(unavailable)?;
    let bucket = source.map_or_else(|| "local".to_owned(), |value| source_bucket(scope, value));
    let account = available(db, scope, "account", now)?;
    let origin = available(db, scope, &bucket, now)?;
    if account.1 >= LIMIT || origin.1 >= LIMIT {
        return Ok(Err(SessionError::RateLimited));
    }
    if source.is_some() {
        let (exists, count): (bool, i64) = db.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE bucket=?1),(SELECT count(*) FROM {table} WHERE bucket LIKE 'ip:%')"),
            [&bucket], |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(unavailable)?;
        if !exists && count >= NETWORK_BUCKETS {
            return Ok(Err(SessionError::Capacity));
        }
    }
    for (bucket, (started, attempts)) in [("account", account), (bucket.as_str(), origin)] {
        db.execute(
            &format!("INSERT INTO {table}(bucket,window_started,attempts) VALUES(?1,?2,?3) ON CONFLICT(bucket) DO UPDATE SET window_started=excluded.window_started,attempts=excluded.attempts"),
            params![bucket, started, attempts + 1],
        ).map_err(unavailable)?;
    }
    Ok(Ok(()))
}

fn available(
    db: &Connection,
    scope: Scope,
    bucket: &str,
    now: i64,
) -> Result<(i64, i64), SessionError> {
    let row: Option<(i64, i64)> = db
        .query_row(
            &format!(
                "SELECT window_started,attempts FROM {} WHERE bucket=?1",
                scope.table()
            ),
            [bucket],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(unavailable)?;
    Ok(match row {
        Some((started, attempts)) if now - started < WINDOW => (started, attempts),
        _ => (now, 0),
    })
}

fn source_bucket(scope: Scope, source: IpAddr) -> String {
    let source = match source {
        IpAddr::V6(address) => address.to_ipv4_mapped().map_or(source, IpAddr::V4),
        IpAddr::V4(_) => source,
    };
    let mut hash = Sha256::new();
    hash.update(scope.domain());
    // TOTP 的旧地址族标签与固定长网络字节编码逐字保持兼容。
    match source {
        IpAddr::V4(address) => {
            hash.update([4]);
            hash.update(address.octets());
        }
        IpAddr::V6(address) => {
            hash.update([6]);
            hash.update(address.octets());
        }
    }
    format!("ip:{}", secrets::hex(&hash.finalize()))
}

pub(super) fn validate(db: &Connection, scope: Scope) -> Result<(), SessionError> {
    let table = scope.table();
    let (count, network, account): (i64, i64, bool) = db.query_row(
        &format!("SELECT count(*),coalesce(sum(bucket LIKE 'ip:%'),0),EXISTS(SELECT 1 FROM {table} WHERE bucket='account') FROM {table}"),
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).map_err(unavailable)?;
    if count > 66 || network > NETWORK_BUCKETS || (count > 0 && !account) {
        return Err(SessionError::CorruptStore);
    }
    let invalid: i64 = db.query_row(
        &format!("SELECT count(*) FROM {table} WHERE typeof(bucket)!='text' OR (bucket NOT IN ('account','local') AND (octet_length(bucket)!=67 OR substr(bucket,1,3)!='ip:' OR substr(bucket,4) GLOB '*[^0-9a-f]*')) OR typeof(window_started)!='integer' OR window_started<0 OR window_started>(SELECT clock_floor FROM auth_meta WHERE id=1) OR typeof(attempts)!='integer' OR attempts NOT BETWEEN 1 AND 5"),
        [], |r| r.get(0),
    ).map_err(unavailable)?;
    if invalid != 0 {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}
