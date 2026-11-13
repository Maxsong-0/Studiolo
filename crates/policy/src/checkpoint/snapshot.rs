//! 对比逻辑内容与业务排序；VACUUM可重编号rowid，但不可改变草案先后关系。
use crate::{
    PolicyError, RevocationLevel,
    database::{Database, schema, unavailable},
    sql,
    store::GLOBAL_TARGET,
};
use rusqlite::{TransactionBehavior, types::ValueRef};
use sha2::{Digest, Sha256};
use std::time::Instant;
use studiolo_core::checkpoint::OwnerWatermark;

#[derive(PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) watermark: OwnerWatermark,
    digest: [u8; 32],
}

pub(super) fn read(database: &Database, deadline: Instant) -> Result<Snapshot, PolicyError> {
    database.until(deadline, |connection| {
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(unavailable)?;
        let version: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(version),0) FROM policy_version",
                [],
                |row| row.get(0),
            )
            .map_err(unavailable)?;
        let watermark = OwnerWatermark::Policy {
            schema_version: u32::try_from(schema::USER_VERSION)
                .map_err(|_| PolicyError::CorruptStore)?,
            effective_version: u64::try_from(version).map_err(|_| PolicyError::CorruptStore)?,
            effective_hash: sql::effective_hash(&tx)?,
            global_revoked: sql::revoked(&tx, RevocationLevel::Global, GLOBAL_TARGET)?,
        };
        let mut hash = Sha256::new();
        // 固定schema下SELECT *列序固定；顺序/长度/类型均入摘要，不拼接有歧义的字符串。
        for query in [
            "SELECT * FROM policy_version ORDER BY version",
            "SELECT * FROM policy_current ORDER BY kind,subject",
            "SELECT * FROM policy_desired ORDER BY rowid",
            "SELECT * FROM policy_revocation ORDER BY seq",
            "SELECT * FROM policy_meta ORDER BY key",
        ] {
            hash.update([0xff]);
            hash.update(query.as_bytes());
            let mut statement = tx.prepare(query).map_err(unavailable)?;
            let columns = statement.column_count();
            let mut rows = statement.query([]).map_err(unavailable)?;
            while let Some(row) = rows.next().map_err(unavailable)? {
                super::check_deadline(deadline)?;
                hash.update([0xfe]);
                for column in 0..columns {
                    match row.get_ref(column).map_err(unavailable)? {
                        ValueRef::Null => hash.update([0]),
                        ValueRef::Integer(value) => {
                            hash.update([1]);
                            hash.update(value.to_le_bytes());
                        }
                        ValueRef::Real(value) => {
                            hash.update([2]);
                            hash.update(value.to_bits().to_le_bytes());
                        }
                        ValueRef::Text(value) | ValueRef::Blob(value) => {
                            hash.update([
                                if matches!(
                                    row.get_ref(column).map_err(unavailable)?,
                                    ValueRef::Text(_)
                                ) {
                                    3
                                } else {
                                    4
                                },
                            ]);
                            hash.update((value.len() as u64).to_le_bytes());
                            hash.update(value);
                        }
                    }
                }
            }
        }
        tx.commit().map_err(unavailable)?;
        Ok(Snapshot {
            watermark,
            digest: hash.finalize().into(),
        })
    })
}
