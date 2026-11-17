//! 编号只根据 Broker 可信扫描的文件名返回上界，不向控制面搬运正文。
use crate::{BrokerError, FsVaultBroker, MemoryVaultBroker, ReadEntryKind, VaultPath};

/// 可信的 canonical 文件名扫描；调用者仍须在数据库事务内串行预留号码。
pub trait NumberingScanner: Send + Sync {
    /// 递归匹配 scope 下 prefix + 十进制数字 + `-` 或 `.` 的普通文件名。
    ///
    /// scope 在 canonical 里不存在时报 [`BrokerError::NotFound`]，不按 0 起算：库里改过目录名、
    /// canonical 还没跟上时，从 1 重新编号会和库里已有的编号撞号（BUG-038）。
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
        // 范围目录不在 canonical 里就不按 0 起算：改名后旧范围扫出 0 会和库里已有编号撞号（BUG-038）。
        let key = scope.collision_key();
        if !batch.entries.iter().any(|entry| {
            entry.kind == ReadEntryKind::Directory && entry.path.collision_key() == key
        }) {
            return Err(BrokerError::NotFound(scope.clone()));
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
        // 内存实现没有单独的目录：scope 下至少有一个对象才算这个目录存在。
        let inside = format!("{}/", scope.collision_key());
        if !self
            .objects
            .keys()
            .any(|path| path.collision_key().starts_with(&inside))
        {
            return Err(BrokerError::NotFound(scope.clone()));
        }
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

    #[test]
    fn a_scope_missing_from_the_vault_is_refused_instead_of_numbering_from_one() {
        let mut broker = MemoryVaultBroker::new();
        broker
            .mutate(Mutation::create(
                VaultPath::parse("项目/甲/06-BUG/BUG-038-x.md").unwrap(),
                b"note".to_vec(),
                "seed",
            ))
            .unwrap();
        // 库里改名成 甲-Docs、这份还没跟上：不能当成空目录从 1 开始。
        let renamed = VaultPath::parse("项目/甲-Docs/06-BUG").unwrap();
        assert!(matches!(
            broker.scan_number_max(&renamed, "BUG-"),
            Err(BrokerError::NotFound(path)) if path == renamed
        ));
        // 文件不是目录，也不算范围存在。
        let file = VaultPath::parse("项目/甲/06-BUG/BUG-038-x.md").unwrap();
        assert!(matches!(
            broker.scan_number_max(&file, "BUG-"),
            Err(BrokerError::NotFound(_))
        ));
        assert_eq!(
            broker
                .scan_number_max(&VaultPath::parse("项目/甲/06-BUG").unwrap(), "BUG-")
                .unwrap(),
            38
        );
    }
}
