//! 只信原 child 的 waitid；不 reap leader 前结束同组子孙，避免 PID 复用后误杀别的进程。
use crate::{ProcessError, Result};
use rustix::process::{self, Pid, Signal, WaitId, WaitIdOptions};
use std::{
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus},
    time::{Duration, Instant},
};

pub(super) struct OwnedChild {
    child: Child,
    pid: Pid,
    owned: bool,
    killed: bool,
    cleanup_attempted: bool,
}
impl OwnedChild {
    pub fn spawn(mut command: Command) -> Result<Self> {
        let mut child = command.spawn().map_err(|_| ProcessError::Spawn)?;
        let Some(pid) = i32::try_from(child.id())
            .ok()
            .filter(|pid| *pid > 1)
            .and_then(Pid::from_raw)
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProcessError::Spawn);
        };
        Ok(Self {
            child,
            pid,
            owned: true,
            killed: false,
            cleanup_attempted: false,
        })
    }
    pub fn stdout(&mut self) -> Result<ChildStdout> {
        self.child.stdout.take().ok_or(ProcessError::Io)
    }
    pub fn stderr(&mut self) -> Result<ChildStderr> {
        self.child.stderr.take().ok_or(ProcessError::Io)
    }
    pub fn stdin(&mut self) -> Result<ChildStdin> {
        self.child.stdin.take().ok_or(ProcessError::Io)
    }
    pub fn exited(&mut self) -> Result<bool> {
        match process::waitid(
            WaitId::Pid(self.pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        ) {
            Ok(status) => Ok(status.is_some()),
            Err(rustix::io::Errno::INTR) => Ok(false),
            Err(_) => {
                // 原 child 的身份已无法证明，绝不再对这个整数 PID 发信号。
                self.owned = false;
                Err(ProcessError::CleanupFailed)
            }
        }
    }
    pub fn kill_group(&mut self) -> Result<()> {
        if !self.owned {
            return Err(ProcessError::CleanupFailed);
        }
        if self.killed {
            return Ok(());
        }
        match process::kill_process_group(self.pid, Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => self.killed = true,
            Err(_) => return Err(ProcessError::CleanupFailed),
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<ExitStatus> {
        self.cleanup_attempted = true;
        self.exited()?;
        self.kill_group()?;
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.exited()? {
            if Instant::now() >= deadline {
                return Err(ProcessError::CleanupFailed);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let status = self.child.wait().map_err(|_| ProcessError::CleanupFailed)?;
        self.owned = false;
        Ok(status)
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.owned && !self.cleanup_attempted {
            let _ = self.finish();
        }
    }
}
