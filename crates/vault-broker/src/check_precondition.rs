//! 只验证 CAS 并持久保留拒绝；匹配检查从不创建 stage、历史或安装许可。

use crate::mutation::{
    self,
    record::{Outcome, Terminal, cas_matches, conflict},
    replay,
};
use crate::writer::disk;
use crate::{BrokerError, FsVaultBroker, Mutation, Revision, sha256};

impl FsVaultBroker {
    /// 为原请求记录实时 CAS 拒绝，不安装文件或推进提交水位。
    ///
    /// 任何未决阶段都原样保留。既有终态仍遵守原指纹；历史成功返回 Ok，
    /// 拒绝、取消或未知返回原错误。无终态且当前匹配也返回 Ok，但它不是
    /// 可缓存的写入许可：调用方必须重新读取匹配版本、核验范围，最终仍经 mutate CAS。
    pub fn check_precondition(&mut self, request: &Mutation) -> Result<(), BrokerError> {
        self.check_precondition_metadata(request, None)
    }

    pub(crate) fn check_precondition_metadata(
        &mut self,
        request: &Mutation,
        metadata: Option<&crate::MutationMetadata>,
    ) -> Result<(), BrokerError> {
        mutation::validate_request(request)?;
        crate::file_operation::guard_legacy_key(self, &sha256(request.idempotency_key.as_bytes()))?;
        crate::maintenance::ensure_idle(self)?;
        mutation::validate_request(request)?;
        crate::recorded::check_existing(self, request, metadata)?;
        let journal = disk::FilePin::capture(&self.data, "writer-state.json", disk::RECORD_LIMIT)?;
        let state = mutation::load(self)?;
        journal.check(&self.data, "writer-state.json", disk::RECORD_LIMIT)?;
        if self.writer.uncertain() || state.legacy_unresolved || state.operation.is_some() {
            return Err(BrokerError::MutationUnresolved);
        }
        let fingerprint = Self::fingerprint(request)?;
        let target = Revision::compute(self.vault_uuid, &request.path, &request.content);
        let hash = sha256(request.idempotency_key.as_bytes());
        if let Some(previous) = replay::load(&self.data, &hash, state.pair_id)? {
            let result = replay::matches(
                previous,
                &fingerprint,
                target,
                &request.path,
                request.precondition,
            )
            .map(|_| ());
            self.writer.check(&self.root, &self.data)?;
            journal.check(&self.data, "writer-state.json", disk::RECORD_LIMIT)?;
            return result;
        }
        let actual = crate::blob::current::observe(self, &request.path)?.revision();
        self.writer.check(&self.root, &self.data)?;
        journal.check(&self.data, "writer-state.json", disk::RECORD_LIMIT)?;
        if cas_matches(request.precondition, actual) {
            return Ok(());
        }
        let existing =
            crate::recorded::store::load_id(self, &crate::recorded::store::id(self, &hash))?;
        let recording = if metadata.is_some() || existing.is_some() {
            Some(crate::recorded::store::prepare(self, request, metadata)?.reference()?)
        } else {
            None
        };
        let terminal = Terminal {
            format_version: if recording.is_some() { 4 } else { 2 },
            recording,
            pair_id: state.pair_id,
            sequence: None,
            fingerprint,
            expected_revision: target,
            outcome: Outcome::Rejected {
                path: request.path.clone(),
                precondition: request.precondition,
                actual,
            },
        };
        mutation::save_terminal(self, &hash, &terminal)?;
        mutation::fault(self, 6)?;
        self.writer.check(&self.root, &self.data)?;
        journal.check(&self.data, "writer-state.json", disk::RECORD_LIMIT)?;
        Err(conflict(request.path.clone(), request.precondition, actual))
    }
}
