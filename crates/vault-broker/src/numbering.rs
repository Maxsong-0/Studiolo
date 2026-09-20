//! 编号只根据 Broker 可信扫描的文件名返回上界，不向控制面搬运正文。
use crate::{BrokerError, FsVaultBroker, MemoryVaultBroker, ReadEntryKind, VaultPath};

/// 可信的 canonical 文件名扫描；调用者仍须在数据库事务内串行预留号码。
pub trait NumberingScanner: Send + Sync {
    /// 递归匹配 scope 下 prefix + 十进制数字 + `-` 或 `.` 的普通文件名。
    fn scan_number_max(&self, scope: &VaultPath, prefix: &str) -> Result<u64, BrokerError>;
}
fn validate(scope: &VaultPath, prefix: &str) -> Result<(), BrokerError> {
    if scope.is_protected() {
        return Err(BrokerError::ProtectedPath(scope.clone()));
    }
    if prefix.is_empty()
        || prefix.len() > 64
        || (prefix.contains(['/', '\\'])
            || prefix.chars().any(char::is_control)
            || prefix.trim() != prefix)
    {
        return Err(BrokerError::InvalidArtifact("编号前缀无效".into()));
    }
    Ok(())
}
fn number(path: &VaultPath, scope: &VaultPath, prefix: &str) -> Result<u64, BrokerError> {
    if !path
        .collision_key()
        .starts_with(&format!("{}/", scope.collision_key()))
    {
        return Ok(0);
    }
    let name = path
        .as_str()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_lowercase();
    let normalized_prefix = prefix.to_lowercase();
    let Some(tail) = name.strip_prefix(&normalized_prefix) else {
        return Ok(0);
    };
    let count = tail.bytes().take_while(u8::is_ascii_digit).count();
    if count == 0 || !matches!(tail.as_bytes().get(count), Some(b'-' | b'.')) {
        return Ok(0);
    }
    tail[..count]
        .parse()
        .map_err(|_| BrokerError::WriterLimitExceeded)
}
impl NumberingScanner for FsVaultBroker {
    fn scan_number_max(&self, scope: &VaultPath, prefix: &str) -> Result<u64, BrokerError> {
        validate(scope, prefix)?;
        let committed = crate::capture::ready(self)?;
        let batch =
            crate::readonly::scan_bound(&self.root, crate::ScanLimits::default(), self.vault_uuid)?;
        self.check_read_watermark(committed)?;
        if !batch.diagnostics.is_empty() {
            return Err(BrokerError::IncompleteCapture);
        }
        let mut max = 0;
        for entry in batch.entries {
            if entry.kind != ReadEntryKind::Directory {
                max = max.max(number(&entry.path, scope, prefix)?);
            }
        }
        Ok(max)
    }
}
impl NumberingScanner for MemoryVaultBroker {
    fn scan_number_max(&self, scope: &VaultPath, prefix: &str) -> Result<u64, BrokerError> {
        validate(scope, prefix)?;
        let mut max = 0;
        for path in self.objects.keys() {
            max = max.max(number(path, scope, prefix)?);
        }
        Ok(max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mutation, VaultWriter};
    #[test]
    fn case_aliases_share_the_same_numbering_space() {
        let mut broker = MemoryVaultBroker::new();
        broker
            .mutate(Mutation::create(
                VaultPath::parse("Tasks/T-012-note.md").unwrap(),
                b"note".to_vec(),
                "seed",
            ))
            .unwrap();
        assert_eq!(
            broker
                .scan_number_max(&VaultPath::parse("tasks").unwrap(), "t-")
                .unwrap(),
            12
        );
        for prefix in ["", "../", "T-\n", " T-", "T-\t", "T\\"] {
            assert!(
                broker
                    .scan_number_max(&VaultPath::parse("Tasks").unwrap(), prefix)
                    .is_err()
            );
        }
    }
}
