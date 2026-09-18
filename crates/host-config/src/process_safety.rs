//! 只收紧当前进程的秘密驻留面，不改变父进程、服务或系统配置。
//!
//! 「秘密驻留面」指种子、会话令牌等明文在本进程里可能被别处读到的四条路径：
//! 崩溃时落盘的核心转储、在线的进程检视入口、被换出到交换区的页，以及
//! 由固定环境槽位传入、随后仍留在系统环境块里的那份副本。
//!
//! 本模块只做一次性收紧，并对每条路径返回固定分类：
//! 本平台提供的能力必须真正生效并回读确认，否则整体拒绝且不读取秘密；
//! 本平台不提供的能力记为明确的「不支持」，绝不静默跳过后宣称已加固。
//! 调用方必须把返回的分类记录下来——只返回不记录，等于把「不支持」藏起来，
//! 与静默跳过没有区别。
//!
//! 三个持有秘密的可执行程序共用这一份实现，各自在最早的无条件路径上调用：
//! `studiolo-auth-admin`（`main::run`）、`studiolo-local-server`
//! （`startup::prepare_composed`，与是否启用因子模式无关）、
//! `studiolo-maintain`（`maintenance::run`）。
//! 本模块住在 host-config，是因为管理命令与服务器共享的安全知识都归这里；
//! 任何新的持秘密程序必须调用它，不得另写一份部分实现。
//!
//! 本模块不宣称抵御 root、调试授权持有者或已经能读本进程内存的一方。

mod platform;
mod report;

#[cfg(test)]
#[path = "../tests/support/process_safety_tests.rs"]
mod tests;

use platform::Capability;
#[cfg(test)]
pub(crate) use report::{Applied, Unsupported};
pub(crate) use report::{Cause, Measure, Outcome};
pub use report::{Refusal, Residency};

/// 在读取任何秘密之前收紧本进程；必须早于任何秘密读取与任何派生子进程。
///
/// 返回逐项固定分类供调用方记录与契约文档核对；重复调用是幂等的。
///
/// # Errors
///
/// 本平台声明支持的能力没能真正生效时返回 [`Refusal`]，调用方必须在未读取
/// 任何秘密的情况下终止，不得降级成「没成功也照跑」。
pub fn harden() -> Result<Residency, Refusal> {
    let core = record(Measure::CoreDumpLimit, platform::core_dump_limit())?;
    let inspection = record(Measure::ProcessInspection, platform::process_inspection())?;
    let memory = record(Measure::MemoryResidency, platform::memory_residency())?;
    let environment = classify(platform::environment_slot(shields_external_read(
        inspection,
    )));
    Ok(Residency::new([core, inspection, memory, environment]))
}

/// 环境块是否已被挡住，只看 [`Measure::ProcessInspection`] 的**实测结果**。
///
/// 该措施只有一个「已生效」取值（关闭可转储属性），所以这里判分类即可；
/// 措施将来退化或被平台拒绝时，环境槽位的结论会自动跟着退到「同用户可读」。
fn shields_external_read(inspection: Outcome) -> bool {
    matches!(inspection, Outcome::Applied(_))
}

fn record(measure: Measure, result: Result<Capability, Cause>) -> Result<Outcome, Refusal> {
    result
        .map(classify)
        .map_err(|cause| Refusal::new(measure, cause))
}

fn classify(capability: Capability) -> Outcome {
    match capability {
        Capability::Applied(applied) => Outcome::Applied(applied),
        Capability::Unsupported(reason) => Outcome::Unsupported(reason),
    }
}
