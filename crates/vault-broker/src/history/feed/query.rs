use super::{ChangePage, source, storage};
use crate::{BrokerError, FsVaultBroker};
pub(in crate::history) fn cursor(through: u64, bucket: u64, offset: usize) -> String {
    format!("f1:{through}:{bucket}:{offset}")
}
pub(in crate::history) fn parse(
    value: &str,
    committed: u64,
) -> Result<(u64, u64, usize), BrokerError> {
    let fields: Vec<_> = value.split(':').collect();
    if fields.len() != 4
        || fields[0] != "f1"
        || fields[1..].iter().any(|v| {
            v.is_empty()
                || v.len() > 20
                || !v.bytes().all(|b| b.is_ascii_digit())
                || (v.len() > 1 && v.starts_with('0'))
        })
    {
        return Err(BrokerError::InvalidHistoryCursor);
    }
    let through = fields[1]
        .parse::<u64>()
        .map_err(|_| BrokerError::InvalidHistoryCursor)?;
    let bucket = fields[2]
        .parse::<u64>()
        .map_err(|_| BrokerError::InvalidHistoryCursor)?;
    let offset = fields[3]
        .parse::<usize>()
        .map_err(|_| BrokerError::InvalidHistoryCursor)?;
    if through == 0
        || through > committed
        || bucket > storage::number(through)
        || offset > storage::ROWS
    {
        return Err(BrokerError::InvalidHistoryCursor);
    }
    Ok((through, bucket, offset))
}
impl FsVaultBroker {
    /// 有界读取Broker提交账，不扫描正文；缺失历史桶如实返回不完整标记。
    pub fn list_changes(
        &self,
        limit: usize,
        before: Option<&str>,
    ) -> Result<ChangePage, BrokerError> {
        if !(1..=100).contains(&limit) {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let state = super::super::query::readable(self)?;
        let (through, mut bucket, mut offset) = before
            .map(|s| parse(s, state.committed_through))
            .transpose()?
            .unwrap_or((
                state.committed_through,
                storage::number(state.committed_through),
                0,
            ));
        let mut page = ChangePage {
            entries: Vec::new(),
            next_cursor: None,
            through_sequence: through,
            history_incomplete: false,
            retired: false,
        };
        if through == 0 {
            return Ok(page);
        }
        for _ in 0..2 {
            let value = storage::load(self, bucket)?;
            if value.is_none() {
                page.history_incomplete = true;
            }
            let rows: Vec<_> = value
                .as_ref()
                .map(|b| b.rows.iter().filter(|r| r.sequence <= through).collect())
                .unwrap_or_default();
            if offset > rows.len() {
                return Err(BrokerError::HistoryChanged);
            }
            while offset < rows.len() {
                let row = rows[offset];
                offset += 1;
                if let Some(item) = storage::item(self, row)? {
                    if super::super::batch::visible(self, &item)? {
                        page.entries.push(source::project(self, &row.path, &item)?);
                    }
                } else {
                    page.retired = true;
                }
                if page.entries.len() == limit {
                    break;
                }
            }
            if offset < rows.len() {
                page.next_cursor = Some(cursor(through, bucket, offset));
                break;
            }
            if bucket == 0 {
                page.next_cursor = None;
                break;
            }
            bucket -= 1;
            offset = 0;
            page.next_cursor = Some(cursor(through, bucket, offset));
            if page.entries.len() == limit {
                break;
            }
        }
        super::super::query::unchanged(self, &state)?;
        Ok(page)
    }
}
