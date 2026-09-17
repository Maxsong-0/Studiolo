//! 历史终态查询不读取当前页面，不清理 pending，也不授予再次写入资格。

use crate::{BrokerError, FsVaultBroker, Mutation, MutationReceipt, Revision, mutation, sha256};

impl FsVaultBroker {
    /// 核对原请求后读取永久回执；页面后来变化或删除不改变历史结果。
    /// 没有终态返回 None；未知安装不按当前内容补成功，已存在的 pending 保持原样。
    pub fn mutation_replay(
        &self,
        request: &Mutation,
    ) -> Result<Option<MutationReceipt>, BrokerError> {
        mutation::validate_request(request)?;
        crate::recorded::check_existing(self, request, None)?;
        let state = mutation::load(self)?;
        if self.writer.uncertain() {
            return Err(BrokerError::MutationUnresolved);
        }
        let fingerprint = Self::fingerprint(request)?;
        let target = Revision::compute(self.vault_uuid, &request.path, &request.content);
        let replay = mutation::replay::load(
            &self.data,
            &sha256(request.idempotency_key.as_bytes()),
            state.pair_id,
        )?;
        let result = replay
            .map(|value| {
                mutation::replay::matches(
                    value,
                    &fingerprint,
                    target,
                    &request.path,
                    request.precondition,
                )
            })
            .transpose();
        self.writer.check(&self.root, &self.data)?;
        // 查询期间提交日志变化不能被读作稳定历史证明；不扫描或观察当前页面。
        let after = mutation::load(self)?;
        if serde_json::to_vec(&state).map_err(|_| BrokerError::CorruptWriterState)?
            != serde_json::to_vec(&after).map_err(|_| BrokerError::CorruptWriterState)?
        {
            return Err(BrokerError::CaptureChanged);
        }
        result
    }
}
