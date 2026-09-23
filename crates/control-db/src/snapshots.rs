//! 只含元数据的不可变输入快照：落盘与读取。
use super::*;

impl ControlDb {
    /// 加载单个快照的元数据。
    pub fn snapshot(&self, snapshot_id: &str) -> Result<Option<SnapshotRecord>, DbError> {
        let vault: Option<String> = self
            .connection
            .query_row(
                "SELECT vault_uuid FROM snapshots WHERE snapshot_id = ?1",
                [snapshot_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(vault) = vault else {
            return Ok(None);
        };
        let mut statement = self.connection.prepare(
            "SELECT path, revision, effective_label_json FROM snapshot_entries
             WHERE snapshot_id = ?1 ORDER BY path",
        )?;
        let rows = statement.query_map([snapshot_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (path, revision, labels) = row?;
            entries.push(SnapshotEntryRecord {
                path,
                revision,
                effective_label_json: serde_json::from_str(&labels)?,
            });
        }
        Ok(Some(SnapshotRecord {
            snapshot_id: snapshot_id.to_owned(),
            vault_uuid: parse_vault_uuid(&vault)?,
            entries,
        }))
    }
}

pub(super) fn persist_snapshot(
    tx: &Transaction<'_>,
    snapshot: &SnapshotRecord,
) -> Result<(), DbError> {
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO snapshots(snapshot_id, vault_uuid) VALUES (?1, ?2)",
        params![&snapshot.snapshot_id, snapshot.vault_uuid.to_string()],
    )?;
    if inserted == 0 {
        let stored: String = tx.query_row(
            "SELECT vault_uuid FROM snapshots WHERE snapshot_id = ?1",
            [&snapshot.snapshot_id],
            |row| row.get(0),
        )?;
        if stored != snapshot.vault_uuid.to_string() {
            return Err(DbError::Corrupt("snapshot id collision".to_owned()));
        }
        let stored_entries = {
            let mut statement = tx.prepare(
                "SELECT path, revision, effective_label_json FROM snapshot_entries
                 WHERE snapshot_id = ?1 ORDER BY path",
            )?;
            let rows = statement.query_map([&snapshot.snapshot_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let mut entries = Vec::new();
            for row in rows {
                let (path, revision, labels) = row?;
                entries.push(SnapshotEntryRecord {
                    path,
                    revision,
                    effective_label_json: serde_json::from_str(&labels)?,
                });
            }
            entries
        };
        let mut requested_entries = snapshot.entries.clone();
        requested_entries.sort_by(|left, right| left.path.cmp(&right.path));
        if stored_entries != requested_entries {
            return Err(DbError::Corrupt("snapshot id collision".to_owned()));
        }
        return Ok(());
    }
    for entry in &snapshot.entries {
        tx.execute(
            "INSERT INTO snapshot_entries(snapshot_id, path, revision, effective_label_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                &snapshot.snapshot_id,
                &entry.path,
                &entry.revision,
                serde_json::to_string(&entry.effective_label_json)?
            ],
        )?;
    }
    Ok(())
}
