//! 平台差异只在这一层分叉；每个分支都给出固定分类，不把缺失能力写成成功。
//!
//! 约定：函数返回 `Ok(Capability::Applied(..))` 只在**真正生效并回读确认**后给出；
//! 本平台根本没有该能力时返回 `Ok(Capability::Unsupported(..))`；
//! 本平台有该能力却没生效时返回 `Err(Cause)`，由调用方转成拒绝并终止。

use super::report::{Applied, Cause, Unsupported};
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

/// 单项措施在本平台上的结论；只有「生效」与「不提供」两种成功取值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Capability {
    Applied(Applied),
    Unsupported(Unsupported),
}

/// 软硬限额同时降为 0 并回读；两个平台都提供该能力，失败即拒绝。
///
/// 与命令无关：迁移、补记、维护、非因子模式的服务器这类不读种子的路径同样禁 core，
/// 因为崩溃转储会连同配置路径、会话令牌与 CSRF 令牌一起落盘。
pub(super) fn core_dump_limit() -> Result<Capability, Cause> {
    let disabled = Rlimit {
        current: Some(0),
        maximum: Some(0),
    };
    setrlimit(Resource::Core, disabled).map_err(|_| Cause::Rejected)?;
    let actual = getrlimit(Resource::Core);
    if actual.current != Some(0) || actual.maximum != Some(0) {
        return Err(Cause::Unconfirmed);
    }
    Ok(Capability::Applied(Applied::CoreDumpLimitZeroed))
}

/// Linux：`PR_SET_DUMPABLE(0)` 并用 `PR_GET_DUMPABLE` 回读。
///
/// 副作用是本措施的主要价值：不可转储的进程，其 `/proc/<pid>` 下
/// `environ`、`mem`、`maps` 等入口会走 `ptrace_may_access` 检查，
/// 同 UID 的其他进程读取会得到 `EACCES`，进程自读不受影响。
/// 持 `CAP_SYS_PTRACE` 的一方（含 root）不受此限制，本模块不宣称挡得住。
#[cfg(target_os = "linux")]
pub(super) fn process_inspection() -> Result<Capability, Cause> {
    use rustix::process::{DumpableBehavior, dumpable_behavior, set_dumpable_behavior};

    set_dumpable_behavior(DumpableBehavior::NotDumpable).map_err(|_| Cause::Rejected)?;
    if dumpable_behavior().map_err(|_| Cause::Unconfirmed)? != DumpableBehavior::NotDumpable {
        return Err(Cause::Unconfirmed);
    }
    Ok(Capability::Applied(Applied::NotDumpable))
}

/// macOS：没有 prctl 的可转储开关，也没有 `/proc` 这条同 UID 读取路径。
///
/// macOS 侧读取他进程内存要经 `task_for_pid`，默认只对 root 或持调试授权者开放，
/// 那是系统策略而不是本进程设置的开关；本进程因此**不**宣称获得了等价结果。
/// 环境块另有 `KERN_PROCARGS2` 一条同 UID 可读路径，见 [`environment_slot`]。
#[cfg(not(target_os = "linux"))]
pub(super) fn process_inspection() -> Result<Capability, Cause> {
    Ok(Capability::Unsupported(Unsupported::NoDumpableSwitch))
}

/// Linux：先试「缺页时锁定」，内核不支持再退回整地址空间预先锁定。
///
/// 选 `mlockall` 而不是按缓冲 `mlock`，是因为按缓冲锁定需要 unsafe（工程禁用），
/// 而且锁定粒度本来就是页：秘密在本进程里有解码缓冲、hex 字符串、HMAC 中间态
/// 等多份短命副本，逐个锁定必然漏掉一片。整地址空间标记是这里唯一不漏的做法。
///
/// 代价写在契约里：本进程的常驻集必须落在 `RLIMIT_MEMLOCK` 之内，
/// 上限不足时**整体拒绝并且不读取秘密**，不降级成「没锁上也照跑」。
/// 长驻的服务器进程常驻集远大于短命 CLI，部署单元必须显式抬高该上限，
/// 见 `docs/m2/secret-residency.md` §5。
#[cfg(target_os = "linux")]
pub(super) fn memory_residency() -> Result<Capability, Cause> {
    use rustix::mm::{MlockAllFlags, mlockall};

    let mapped = MlockAllFlags::CURRENT | MlockAllFlags::FUTURE;
    match mlockall(mapped | MlockAllFlags::ONFAULT) {
        Ok(()) => return Ok(Capability::Applied(Applied::LockedOnFault)),
        // 4.4 之前的内核没有缺页锁定；退回预先锁定，不冒称已按缺页生效。
        Err(rustix::io::Errno::INVAL) => {}
        Err(error) => return Err(lock_cause(error)),
    }
    mlockall(mapped).map_err(lock_cause)?;
    Ok(Capability::Applied(Applied::LockedEagerly))
}

#[cfg(target_os = "linux")]
fn lock_cause(error: rustix::io::Errno) -> Cause {
    match error {
        // 两者都表示可锁定内存上限不够，操作者需要提高 RLIMIT_MEMLOCK。
        rustix::io::Errno::NOMEM | rustix::io::Errno::PERM => Cause::MemoryLockLimit,
        _ => Cause::Rejected,
    }
}

/// macOS：`mlockall` 不可用，按页 `mlock` 需要 unsafe；本进程不宣称防换出。
#[cfg(not(target_os = "linux"))]
pub(super) fn memory_residency() -> Result<Capability, Cause> {
    Ok(Capability::Unsupported(Unsupported::NoAddressSpaceLock))
}

/// 环境槽位的结论**由本次实测的可检视属性推出**，不按平台假定。
///
/// 两个平台一致的部分：系统环境副本不清除，只保证本进程自有副本零化。
/// 这是**固定结论**而不是失败：`std::env::remove_var` 在 edition 2024 是 unsafe
/// 且工程禁用 unsafe；即便允许，它也只摘掉 libc 的指针表，不改写内核在初始栈上
/// 保留的那份字符串，Linux 上 `/proc/<pid>/environ` 仍会原样读出。
///
/// 两个平台不同的部分正是 `shielded` 这一位：Linux 上可检视属性已经生效，
/// 同 UID 的外部读取被挡住；macOS 上没有这道开关，同用户仍读得到。
/// 参数取自 [`process_inspection`] 的真实结果，措施退化时这里跟着退化。
pub(super) fn environment_slot(shielded: bool) -> Capability {
    Capability::Unsupported(if shielded {
        Unsupported::EnvironmentBlockShieldedByInspection
    } else {
        Unsupported::EnvironmentBlockReadableBySameUser
    })
}
