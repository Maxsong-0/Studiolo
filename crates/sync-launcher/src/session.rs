//! launcher 侧的 session 状态机（纯逻辑，无 IO）：一个 session 对应一次 `launch`，
//! 放行只消费一次，`gate` 后永不再放行；旧 session 只有 reap 后且子组为空才能被新 launch 替换。

use studiolo_sync_transport::linux::protocol::{ContainerFrame, Verdict};
use studiolo_sync_transport::owned_run::RunBudget;

/// 阶段；每次转换都由 `serve.rs` 在真实 OS 事实之后调用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// unit 已起、垫片已报回 (dev,ino)、放行 FIFO 未写。
    Barrier,
    /// 已写放行字节；payload 已 exec 固定程序。
    Released,
    /// spawn 入口已关闭：本 session 不再接受任何放行。
    Gated,
    /// 已读到 `ExecMainStatus`／`ExecMainCode`。
    Reaped,
    /// 已 `systemctl stop` 卸载；session 目录已清。
    Retired,
}

/// 一个 session 的全部记忆。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// 容器身份（launcher 自己测出来的）。
    pub container: ContainerFrame,
    /// 本次运行预算。
    pub budget: RunBudget,
    /// 当前阶段。
    pub phase: Phase,
    /// 已排空的 stdout 字节。
    pub stdout_bytes: u64,
    /// 已排空的 stderr 字节。
    pub stderr_bytes: u64,
    /// 是否触及输出限额。
    pub output_over_budget: bool,
    /// reap 后的退出码。
    pub exit_code: Option<i32>,
    /// reap 后的信号。
    pub exit_signal: Option<i32>,
}

impl Session {
    /// 屏障建立后的初始 session。
    #[must_use]
    pub fn barrier(container: ContainerFrame, budget: RunBudget) -> Self {
        Self {
            container,
            budget,
            phase: Phase::Barrier,
            stdout_bytes: 0,
            stderr_bytes: 0,
            output_over_budget: false,
            exit_code: None,
            exit_signal: None,
        }
    }

    /// 请求帧里的身份必须与本 session 逐字段相同。
    ///
    /// # Errors
    /// 任一字段不同返回 `IdentityMismatch`。
    pub fn check(&self, frame: &ContainerFrame) -> Result<(), Verdict> {
        if frame == &self.container {
            Ok(())
        } else {
            Err(Verdict::IdentityMismatch)
        }
    }

    /// 放行：只在 `Barrier` 允许，之后进入 `Released`。
    ///
    /// # Errors
    /// 已放行或已关门返回 `Busy`。
    pub fn release(&mut self) -> Result<(), Verdict> {
        if self.phase != Phase::Barrier {
            return Err(Verdict::Busy);
        }
        self.phase = Phase::Released;
        Ok(())
    }

    /// 关门：`Barrier`／`Released` 都可关；关门后放行永远被拒。
    pub fn gate(&mut self) {
        if matches!(self.phase, Phase::Barrier | Phase::Released) {
            self.phase = Phase::Gated;
        }
    }

    /// 登记真实等待结果；只在 `Gated` 之后。
    ///
    /// # Errors
    /// 未关门就 reap 返回 `Busy`。
    pub fn reap(
        &mut self,
        exit_code: Option<i32>,
        exit_signal: Option<i32>,
    ) -> Result<(), Verdict> {
        if self.phase != Phase::Gated {
            return Err(Verdict::Busy);
        }
        self.exit_code = exit_code;
        self.exit_signal = exit_signal;
        self.phase = Phase::Reaped;
        Ok(())
    }

    /// 是否可以被新的 launch 替换（已 reap；子组是否为空由调用方另证）。
    #[must_use]
    pub fn replaceable(&self) -> bool {
        matches!(self.phase, Phase::Reaped | Phase::Retired)
    }
}
