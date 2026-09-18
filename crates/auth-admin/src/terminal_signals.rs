//! 单用途 CLI 拥有这些信号：输入期间只置取消标志，恢复终端后恢复默认行为。

use super::TerminalError;
use signal_hook::{
    consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGTSTP},
    flag,
};
use std::sync::{
    Arc, Mutex, MutexGuard, OnceLock, TryLockError,
    atomic::{AtomicBool, Ordering},
};

struct State {
    cancelled: Arc<AtomicBool>,
    defaults: Arc<AtomicBool>,
}

static STATE: OnceLock<Result<State, TerminalError>> = OnceLock::new();
// 进程级读入闸门：只排他，守卫是 `()`，不保护任何共享可变状态。
static READER: Mutex<()> = Mutex::new(());

fn state() -> Result<&'static State, TerminalError> {
    STATE
        .get_or_init(|| {
            let state = State {
                cancelled: Arc::new(AtomicBool::new(false)),
                defaults: Arc::new(AtomicBool::new(true)),
            };
            for signal in [SIGINT, SIGTERM, SIGHUP, SIGQUIT, SIGTSTP] {
                // 先安装默认行为；部分注册失败时也不让这些信号被永久吞掉。
                let _ = flag::register_conditional_default(signal, state.defaults.clone())
                    .map_err(|_| TerminalError::Signal)?;
                let _ = flag::register(signal, state.cancelled.clone())
                    .map_err(|_| TerminalError::Signal)?;
            }
            Ok(state)
        })
        .as_ref()
        .map_err(|error| *error)
}

pub(super) fn initialize() -> Result<(), TerminalError> {
    state().map(|_| ())
}

pub(super) struct Capture {
    state: &'static State,
    _reader: MutexGuard<'static, ()>,
}

impl Capture {
    pub fn begin() -> Result<Self, TerminalError> {
        let reader = match READER.try_lock() {
            Ok(reader) => reader,
            // 闸门无不变量可破坏：上一位持有者 panic 留下的毒化必须可接管，否则一次
            // panic 会让本进程此后每次读入都永久返回 Busy。Capture 的 Drop 在展开时
            // 已恢复默认信号行为，接管时状态是一致的。
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(TerminalError::Busy),
        };
        let state = state()?;
        state.cancelled.store(false, Ordering::SeqCst);
        state.defaults.store(false, Ordering::SeqCst);
        Ok(Self {
            state,
            _reader: reader,
        })
    }

    pub fn check(&self) -> Result<(), TerminalError> {
        if self.state.cancelled.load(Ordering::SeqCst) {
            Err(TerminalError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn finish(self) -> Result<(), TerminalError> {
        // 先恢复默认行为再检查已收信号，避免检查后到返回之间吞掉新来的 Ctrl-C。
        self.state.defaults.store(true, Ordering::SeqCst);
        self.check()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // signal-hook 的 unregister 不恢复默认 handler，因此保留固定的进程级门控。
        // 调用者必须先释放终端恢复守卫；SIGKILL/SIGSTOP 无法拦截，不在保证内。
        self.state.defaults.store(true, Ordering::SeqCst);
    }
}
