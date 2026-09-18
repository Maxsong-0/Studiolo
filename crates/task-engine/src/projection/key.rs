//! 投影幂等键：键本身编码「哪个事件、基于哪个基态、写成什么、第几代」。
//!
//! Broker 把每个幂等键绑死在一次请求的终态上：同请求重放返回首次结果，明确 CAS 拒绝
//! 永久保持拒绝（`docs/contracts/vault-broker.md` §3「明确CAS拒绝保持拒绝」）。所以键必须
//! 同时满足两件事：
//!
//! 1. **同一请求的重试落在同一个键上**，否则一次崩溃窗口里的重试会被当成新请求；
//! 2. **任何真正不同的写入落在不同的键上**，否则它会静默继承旧请求的终态——包括把一次
//!    本该重新评估的写入永久变成「已被拒绝」。
//!
//! 只写事件序号与基态的旧键两条都不满足：页面回到同一基态（外部删除后重新建页、或改
//! 回原内容这种 ABA）就会重新构造出同一个键，而 Broker 只会重放上一次的拒绝。因此键里
//! 同时带上目标修订（正文指纹）与判别位 `generation`。

use studiolo_core::{Revision, TaskId, VaultPath, VaultUuid};
use studiolo_vault_broker::Precondition;

/// 构造一次任务页投影写入的幂等键。
///
/// `generation` 是判别位：它只在调用方重新读取、证明本次前置条件当下仍然成立（即收到的
/// 拒绝只可能是旧终态的重放而不是对当前状态的判定）之后才推进，从而把「同一请求的重试」
/// 与「恰好回到同一基态的另一次写入」分开。
pub(crate) fn mutation_key(
    vault_uuid: VaultUuid,
    task_id: TaskId,
    sequence: i64,
    page: &VaultPath,
    precondition: Precondition,
    content: &[u8],
    generation: u32,
) -> String {
    let base = match precondition {
        Precondition::MustNotExist => "absent".to_owned(),
        Precondition::Match(revision) => revision.to_hex(),
    };
    // 目标修订把请求正文编进键：同键必定同请求，异请求必定异键。
    let target = Revision::compute(vault_uuid, page, content).to_hex();
    format!("task-projection:{task_id}:{sequence}:{base}:{target}:g{generation}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (VaultUuid, TaskId, VaultPath) {
        let task_id = TaskId::new();
        let page = VaultPath::parse(format!("ops/tasks/{task_id}.md")).expect("page path is valid");
        (VaultUuid::new(), task_id, page)
    }

    #[test]
    fn the_same_projection_request_always_produces_the_same_key() {
        let (vault, task, page) = fixture();
        let first = mutation_key(
            vault,
            task,
            7,
            &page,
            Precondition::MustNotExist,
            b"content",
            0,
        );
        let second = mutation_key(
            vault,
            task,
            7,
            &page,
            Precondition::MustNotExist,
            b"content",
            0,
        );
        assert_eq!(first, second);
    }

    #[test]
    fn a_different_target_content_never_reuses_another_requests_key() {
        let (vault, task, page) = fixture();
        let first = mutation_key(vault, task, 7, &page, Precondition::MustNotExist, b"one", 0);
        let second = mutation_key(vault, task, 7, &page, Precondition::MustNotExist, b"two", 0);
        assert_ne!(
            first, second,
            "同一事件的两次不同正文必须落在不同的幂等键上"
        );
    }

    #[test]
    fn a_different_base_state_never_reuses_another_requests_key() {
        let (vault, task, page) = fixture();
        let revision = Revision::compute(vault, &page, b"base");
        let created = mutation_key(vault, task, 7, &page, Precondition::MustNotExist, b"x", 0);
        let replaced = mutation_key(
            vault,
            task,
            7,
            &page,
            Precondition::Match(revision),
            b"x",
            0,
        );
        assert_ne!(created, replaced);
    }

    #[test]
    fn advancing_the_discriminant_mints_a_key_the_broker_has_not_sealed() {
        let (vault, task, page) = fixture();
        let sealed = mutation_key(vault, task, 7, &page, Precondition::MustNotExist, b"x", 0);
        let fresh = mutation_key(vault, task, 7, &page, Precondition::MustNotExist, b"x", 1);
        assert_ne!(sealed, fresh);
    }
}
