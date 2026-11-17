//! 新根恢复的启动闸门（T-212）：规则和已知限制写在 lib 的 `checkpoint::require_restore_revoked`，
//! 那里能拿真实恢复出来的新根测；这里只把拒绝映射成固定的启动错误。
use super::StartupError;
use std::path::Path;
use studiolo_auth::SessionManager;
use studiolo_local_server::checkpoint::require_restore_revoked;

pub(super) fn check(
    writer_data: Option<&Path>,
    sessions: &SessionManager,
) -> Result<(), StartupError> {
    require_restore_revoked(writer_data, sessions)
        .map_err(|_| StartupError::RestoreRevocationPending)
}
