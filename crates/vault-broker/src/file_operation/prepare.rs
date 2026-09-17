//! Preparing 先占唯一主槽；大对象保全结束之后才发出可消费的 Prepared 票据。
use super::{observation::Observation, record::Active, *};
use crate::{BrokerError, FsVaultBroker, mutation};
impl FsVaultBroker {
    /// 只读已初始化的文件操作限额；校验完整主日志与根绑定，不创建存储或补记审计。
    /// 未决时仍可读取，Some 不表示此时可写或已获得当前人侧批准。
    pub fn file_operation_limits(&self) -> Result<Option<FileOperationLimits>, BrokerError> {
        let state = mutation::load(self)?;
        if state.file_operations_enabled {
            Ok(Some(store::required(self)?.limits))
        } else {
            Ok(None)
        }
    }

    /// 显式初始化固定文件操作存储；未知/部分初始化不隐式修复，已启用的限额不得改变。
    pub fn enable_file_operations(
        &mut self,
        limits: FileOperationLimits,
    ) -> Result<(), BrokerError> {
        limits.validate()?;
        crate::maintenance::ensure_idle(self)?;
        let mut state = mutation::load(self)?;
        if state.file_operations_enabled {
            return if store::required(self)?.limits == limits {
                Ok(())
            } else {
                Err(BrokerError::WriterBindingMismatch)
            };
        }
        if self.write_status()?.unresolved || self.content_audit_status()?.restore_unconfirmed {
            return Err(BrokerError::MutationUnresolved);
        }
        schema::initialize(self, limits)?;
        state.file_operations_enabled = true;
        state.format_version = 5;
        mutation::save(self, &state)
    }
    /// 固定请求、实际旧状态与审计意图；返回票据可能已经固定为 Rejected，不能视为执行许可。
    pub fn prepare_file_operation(
        &mut self,
        request: FileOperationRequest,
        metadata: FileOperationMetadata,
    ) -> Result<FileOperationTicket, BrokerError> {
        crate::maintenance::ensure_no_maintenance(self)?;
        let limits = store::required(self)?.limits;
        let frozen = frozen::Frozen::new(self, &request, &metadata, limits)?;
        let mut state = mutation::load(self)?;
        if frozen.version == 2 && !state.rename_enabled {
            return Err(BrokerError::InvalidArtifact("重命名尚未显式启用".into()));
        }
        if let Some(terminal) = terminal::load(self, &frozen)? {
            return ticket::from_terminal(self, &terminal);
        }
        if let Some(active) = &state.file_operation {
            if active.frozen.key_hash == frozen.key_hash && !active.frozen.matches(&frozen) {
                return Err(BrokerError::IdempotencyConflict);
            }
            if !active.frozen.matches(&frozen) || active.phase != FileOperationPhase::Prepared {
                return Err(BrokerError::MutationUnresolved);
            }
            if !active.rejected {
                prepared::check(self, active)?;
            }
            return ticket::from_active(self, active);
        }
        super::legacy_key(self, &frozen.key_hash)?;
        if self.write_status()?.unresolved || self.content_audit_status()?.restore_unconfirmed {
            return Err(BrokerError::MutationUnresolved);
        }
        if request::load(self, &frozen.operation_id)?.is_some() {
            return Err(BrokerError::CorruptWriterState);
        }
        let source = match &frozen.kind {
            FileOperationKind::RestoreTombstone { tombstone_id } => {
                Some(tombstone::source(self, tombstone_id, &frozen.path)?)
            }
            _ => None,
        };
        if matches!(frozen.kind, FileOperationKind::Rename { .. }) {
            return super::rename_prepare::prepare(self, state, frozen);
        }
        let before = Observation::observe(self, &frozen.path)?;
        capacity::layout(&frozen)?;
        let rejected = !match (&frozen.kind, before.presence()?) {
            (FileOperationKind::Delete { expected }, FilePresence::Present { revision, .. }) => {
                *expected == revision
            }
            (FileOperationKind::RestoreTombstone { .. }, FilePresence::Missing) => true,
            _ => false,
        };
        let body = if rejected {
            0
        } else if let Some(source) = &source {
            source.entry()?.bytes
        } else {
            before
                .proof()
                .ok_or(BrokerError::CorruptWriterState)?
                .identity
                .bytes
        };
        let reserved_bytes = capacity::reserve(
            self,
            body,
            matches!(frozen.kind, FileOperationKind::Delete { .. }),
            !rejected,
        )?;
        crate::history::store::open(self, true)?;
        crate::history::preflight(self, &frozen.path)?;
        before.check(self, &frozen.path)?;
        let active = Active {
            frozen,
            sequence: state.next_sequence,
            reserved_bytes,
            phase: FileOperationPhase::Preparing,
            before,
            rejected,
            history: None,
            stage: None,
            sealed_stage: None,
            executed: None,
            quarantine: None,
            effect_at_ms: None,
            terminal: None,
            rename: None,
        };
        state.next_sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        state.file_operation = Some(active);
        mutation::save(self, &state)?;
        mutation::fault(self, 50)?;
        let active = state
            .file_operation
            .as_mut()
            .ok_or(BrokerError::CorruptWriterState)?;
        request::ensure(self, active)?;
        if !active.rejected {
            prepared::preserve(self, active, source.as_ref())?;
        }
        active.phase = FileOperationPhase::Prepared;
        mutation::save(self, &state)?;
        mutation::fault(self, 51)?;
        ticket::from_active(
            self,
            state
                .file_operation
                .as_ref()
                .ok_or(BrokerError::CorruptWriterState)?,
        )
    }
}
