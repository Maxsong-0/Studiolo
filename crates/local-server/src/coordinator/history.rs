//! 历史仍由唯一 canonical owner 读取，按原 Vault 和正文范围检查。
use crate::{CanonicalWorkbench, auth_http::ContentPrincipal, editing::Problem};
use studiolo_core::{Revision, VaultPath, VaultUuid};
use studiolo_vault_broker::{HistoryContent, HistoryPage, VaultReader};

impl CanonicalWorkbench {
    pub(crate) fn history_page(
        &self,
        principal: ContentPrincipal,
        vault: VaultUuid,
        path: &VaultPath,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<HistoryPage, Problem> {
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
