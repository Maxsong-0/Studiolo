//! 操作者说明不参与审计编码；未观察、缺失和不能观察保留各自事实边界。

use super::MaintenanceError;
use serde_json::{Value, json};
use studiolo_vault_broker::{
    FileOperationKind, FileOperationStatus, MaintenanceAuditEvent, MaintenanceCurrent,
    MaintenanceDecision, MaintenanceOutcome, MaintenanceTarget, MaintenanceUnavailableReason,
};

pub(super) fn target(
    target: &MaintenanceTarget,
    operation: Option<&FileOperationStatus>,
) -> Result<Value, MaintenanceError> {
    let original = match target.file_operation_id.as_deref() {
        None => json!({"kind":"legacy_content","description":"原正文写入或页历史恢复"}),
        Some(id) => {
            let status = operation
                .filter(|status| {
                    status.operation_id == id
                        && status.preview.operation_id == id
                        && status.preview.path == target.path
                })
                .ok_or(MaintenanceError::Broker)?;
            original(status)
        }
    };
    let mut note = json!({
        "review_id":target.review_id,"original_operation":original,
        "current_observability":current(target.current),
        "target_revision_note":"target_revision 为 null 表示没有固定的拟写正文版本；不把它解释为空文件或当前缺失。",
        "preservation_note":"取消准备态只取消原请求；保全安装态只封存可验证证据并令原请求永久未知。不能观察的当前对象不会被声称已复制或已删除。"
    });
    if target.secondary.is_some() || target.file_operation.is_some() {
        super::rename::extend(target, operation, &mut note)?;
    } else if operation
        .is_some_and(|status| matches!(status.preview.kind, FileOperationKind::Rename { .. }))
    {
        return Err(MaintenanceError::Broker);
    }
    Ok(note)
}

pub(super) fn original(status: &FileOperationStatus) -> Value {
    let description = match status.preview.kind {
        FileOperationKind::Delete { .. } => "原操作：删除；CAS 绑定原版本，目标状态为缺失",
        FileOperationKind::Rename { .. } => {
            "原操作：重命名；目标只允许原本缺失，先安装目标再摘除源；部分效果不等于整体成功"
        }
        FileOperationKind::RestoreTombstone { .. } => {
            "原操作：从指定墓碑恢复原路径；目标必须原本缺失"
        }
    };
    json!({"kind":status.preview.kind,"phase":status.phase,"description":description,
        "boundary":"固定请求和终态回执不是此刻磁盘状态；审计待确认不等于完成。"})
}

pub(super) fn current(value: MaintenanceCurrent) -> &'static str {
    match value {
        MaintenanceCurrent::NotObserved => "未观察：本次审阅不读取当前正文，不说明当前存在或缺失。",
        MaintenanceCurrent::Missing => {
            "已观察为缺失：这是本次审阅的路径状态，不证明原删除操作成功。"
        }
        MaintenanceCurrent::Present { .. } => {
            "已完整观察当前文件：仅确认审阅时的版本，不证明原操作曾成功。"
        }
        MaintenanceCurrent::Unobservable {
            reason: MaintenanceUnavailableReason::UnsafeOrUnstable,
        } => "不能可靠观察：当前节点不安全或持续变化；不是缺失，也不是删除成功。",
        MaintenanceCurrent::Unobservable {
            reason: MaintenanceUnavailableReason::LimitExceeded,
        } => "不能可靠观察：已超过固定观察预算；不是缺失，也不是删除成功。",
        MaintenanceCurrent::Unobservable {
            reason: MaintenanceUnavailableReason::IoUnavailable,
        } => "不能可靠观察：当前 I/O 不可用；不是缺失，也不是删除成功。",
    }
}

pub(super) fn result(event: &MaintenanceAuditEvent) -> &'static str {
    match (event.outcome, event.decision) {
        (Some(MaintenanceOutcome::Applied), MaintenanceDecision::PreserveInstalling) => {
            "维护保全已形成固定证据包，原请求永久未知；未确认删除、恢复或重命名整体成功，也未替换当前可见对象。"
        }
        (Some(MaintenanceOutcome::Applied), _) => {
            "维护已取消尚未执行的原请求；没有据此删除或安装目标文件。"
        }
        (Some(MaintenanceOutcome::Cancelled), _) => {
            "只取消维护本身；原请求仍需单独核对，不据此宣布原请求取消或成功。"
        }
        _ => "本次维护未协调原请求；请重新审阅固定状态。",
    }
}
