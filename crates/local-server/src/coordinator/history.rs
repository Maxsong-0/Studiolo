//! 历史仍由唯一 canonical owner 读取，按原 Vault 和正文范围检查。
use crate::{
    CanonicalWorkbench,
    auth_http::ContentPrincipal,
    editing::{MAX_BYTES, Problem},
    history::blame,
};
use axum::http::StatusCode;
use std::sync::Arc;
use studiolo_core::{Revision, VaultPath, VaultUuid};
use studiolo_vault_broker::{BrokerError, HistoryContent, HistoryPage, VaultReader};

fn blame_too_large() -> Problem {
    Problem::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "too_large",
        "文档或它的历史版本超出逐行归属的限制，不提供逐行归属。",
    )
}
fn blame_read_error(error: BrokerError) -> Problem {
    match error {
        BrokerError::NotFound(_) => Problem::new(
            StatusCode::NOT_FOUND,
            "history_not_found",
            "文档不存在，或它的历史版本已按保留策略清理。",
        ),
        BrokerError::ReadLimitExceeded { .. } => blame_too_large(),
        other => crate::history::read_error(other),
    }
}

impl CanonicalWorkbench {
    pub(crate) fn history_page(
        &self,
        principal: ContentPrincipal,
        vault: VaultUuid,
        path: &VaultPath,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<HistoryPage, Problem> {
        let _operation = principal
            .enter_operation()
            .map_err(super::content::session_error)?;
        let owner = self.core.owner.lock().map_err(|_| Problem::unavailable())?;
        let broker = owner.as_ref().ok_or_else(Problem::unavailable)?;
        self.check_history_access(&principal, vault, path)?;
        if broker.vault_uuid() != vault {
            return Err(Problem::forbidden());
        }
        let result = broker
            .list_history(path, limit, cursor)
            .map_err(crate::history::read_error)?;
        self.check_history_access(&principal, vault, path)?;
        Ok(result)
    }

    pub(crate) fn history_file(
        &self,
        principal: ContentPrincipal,
        vault: VaultUuid,
        path: &VaultPath,
        revision: Revision,
    ) -> Result<HistoryContent, Problem> {
        let _operation = principal
            .enter_operation()
            .map_err(super::content::session_error)?;
        let owner = self.core.owner.lock().map_err(|_| Problem::unavailable())?;
        let broker = owner.as_ref().ok_or_else(Problem::unavailable)?;
        self.check_history_access(&principal, vault, path)?;
        if broker.vault_uuid() != vault {
            return Err(Problem::forbidden());
        }
        let result = broker
            .read_history(path, revision, crate::editing::MAX_BYTES as u64)
            .map_err(crate::history::read_error)?;
        self.check_history_access(&principal, vault, path)?;
        Ok(result)
    }

    /// 逐行归属要把几十版正文逐版读进来做差分：交给专用派生线程串行做，临时分配都落在
    /// 同一个 arena 里，不随阻塞池换线程（见 `deriver`，BUG-034）。调用方不持有任何锁。
    /// 查询许可交进来随作业释放：投递后客户端断开，作业仍会跑完，许可不能先还。
    pub(crate) fn history_blame(
        self: &Arc<Self>,
        principal: ContentPrincipal,
        vault: Option<VaultUuid>,
        path: VaultPath,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> impl Future<Output = Result<serde_json::Value, Problem>> + Send + use<> {
        let workbench = Arc::clone(self);
        let work = self.core.deriver.run_leased(permit, move |_on| {
            workbench.blame_now(&principal, vault, &path)
        });
        async move { work.await.map_err(|_| Problem::unavailable())? }
    }

    fn blame_now(
        &self,
        principal: &ContentPrincipal,
        requested: Option<VaultUuid>,
        path: &VaultPath,
    ) -> Result<serde_json::Value, Problem> {
        let _operation = principal
            .enter_operation()
            .map_err(super::content::session_error)?;
        let owner = self.core.owner.lock().map_err(|_| Problem::unavailable())?;
        let broker = owner.as_ref().ok_or_else(Problem::unavailable)?;
        let vault = broker.vault_uuid();
        if requested.is_some_and(|value| value != vault) {
            return Err(Problem::forbidden());
        }
        self.check_history_access(principal, vault, path)?;
        let current = broker
            .read_file(path, MAX_BYTES as u64)
            .map_err(blame_read_error)?;
        let lineage = broker
            .history_lineage(path, blame::MAX_VERSIONS)
            .map_err(blame_read_error)?;
        let invalid = || {
            Problem::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_document",
                "文档或它的历史版本不是有效的UTF-8文档。",
            )
        };
        let text = String::from_utf8(current.content).map_err(|_| invalid())?;
        let newest = lineage
            .entries
            .first()
            .is_some_and(|version| version.entry.revision == current.revision);
        let origins = blame::attribute(
            &text,
            newest,
            lineage.entries.len(),
            |n| lineage.entries[n].created,
            |n| {
                let version = broker
                    .read_history(path, lineage.entries[n].entry.revision, MAX_BYTES as u64)
                    .map_err(blame_read_error)?;
                String::from_utf8(version.content).map_err(|_| invalid())
            },
        )
        .map_err(|failure| match failure {
            blame::Failure::TooLarge => blame_too_large(),
            blame::Failure::Read(problem) => problem,
        })?;
        // 算的过程中正文被换掉（库外直接改文件）就不给对不上的结果。
        let after = broker
            .read_file(path, MAX_BYTES as u64)
            .map_err(blame_read_error)?;
        if after.revision != current.revision {
            return Err(crate::history::read_error(BrokerError::HistoryChanged));
        }
        self.check_history_access(principal, vault, path)?;
        Ok(blame::response(
            vault,
            path,
            current.revision,
            &text,
            &lineage,
            &origins,
        ))
    }

    fn check_history_access(
        &self,
        principal: &ContentPrincipal,
        vault: VaultUuid,
        path: &VaultPath,
    ) -> Result<(), Problem> {
        principal.verify().map_err(super::content::session_error)?;
        self.core
            .roots
            .revalidate()
            .map_err(|_| Problem::unavailable())?;
        if self
            .core
            .control
            .lock()
            .map_err(|_| Problem::unavailable())?
            .closing
        {
            return Err(Problem::unavailable());
        }
        if !self.editing_enabled()
            || self
                .core
                .roots
                .human_content()
                .is_none_or(|scope| !scope.allows(vault, path))
        {
            return Err(Problem::forbidden());
        }
        Ok(())
    }
}
