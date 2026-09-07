//! 隐藏输入与显式恢复；失败时返回错误，不把 Drop 的尽力恢复冒充成功。

use super::TerminalError;
use rustix::termios::{
    self, LocalModes, OptionalActions, QueueSelector, SpecialCodeIndex as Code, Termios,
};
use std::fs::File;

pub(super) struct Hidden {
    pub file: File,
    original: Termios,
    active: bool,
}

impl Hidden {
    pub fn enter(file: File) -> Result<Self, TerminalError> {
        let original = termios::tcgetattr(&file).map_err(|_| TerminalError::Io)?;
        let mut hidden = original.clone();
        hidden.make_raw();
        hidden.local_modes.insert(LocalModes::ISIG);
        hidden.special_codes[Code::VMIN] = 0;
        hidden.special_codes[Code::VTIME] = 1;
        hidden.special_codes[Code::VINTR] = 3;
        hidden.special_codes[Code::VQUIT] = 28;
        hidden.special_codes[Code::VSUSP] = 26;
        // 守卫先成立，再更改终端；tcsetattr 部分失败也要尝试恢复。
        let guard = Self {
            file,
            original,
            active: true,
        };
        termios::tcflush(&guard.file, QueueSelector::IFlush).map_err(|_| TerminalError::Io)?;
        termios::tcsetattr(&guard.file, OptionalActions::Now, &hidden)
            .map_err(|_| TerminalError::Io)?;
        let actual = termios::tcgetattr(&guard.file).map_err(|_| TerminalError::Io)?;
        if !same_mode(&actual, &hidden) {
            return Err(TerminalError::Io);
        }
        Ok(guard)
    }

    pub fn restore(&mut self) -> Result<(), TerminalError> {
        if !self.active {
            return Ok(());
        }
        // 不等待输出 drain；先丢弃未读取输入，避免多余验证码遗留到下一条 shell 命令。
        let flushed = termios::tcflush(&self.file, QueueSelector::IFlush);
        let restored = termios::tcsetattr(&self.file, OptionalActions::Now, &self.original);
        let mut actual = termios::tcgetattr(&self.file).map_err(|_| TerminalError::Restore)?;
        if flushed.is_ok() && restored.is_ok() && canonical_transition(&actual, &self.original) {
            // XNU 在 raw→canonical 的 TCSANOW 上设置 PENDIN；IFlush 清除此状态。
            // 固定重设一次后仍完整核对，绝不把 PENDIN 或回显位从最终比较中删去。
            termios::tcflush(&self.file, QueueSelector::IFlush)
                .map_err(|_| TerminalError::Restore)?;
            termios::tcsetattr(&self.file, OptionalActions::Now, &self.original)
                .map_err(|_| TerminalError::Restore)?;
            actual = termios::tcgetattr(&self.file).map_err(|_| TerminalError::Restore)?;
        }
        if restored.is_err() || !same_mode(&actual, &self.original) {
            return Err(TerminalError::Restore);
        }
        self.active = false;
        flushed.map_err(|_| TerminalError::Restore)
    }
}

impl Drop for Hidden {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

pub(super) fn canonical_transition(actual: &Termios, expected: &Termios) -> bool {
    #[cfg(target_os = "macos")]
    {
        if !expected.local_modes.contains(LocalModes::ICANON)
            || (actual.local_modes ^ expected.local_modes) != LocalModes::PENDIN
        {
            return false;
        }
        let mut settled = actual.clone();
        settled.local_modes.remove(LocalModes::PENDIN);
        same_mode(&settled, expected)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (actual, expected);
        false
    }
}

fn same_mode(actual: &Termios, expected: &Termios) -> bool {
    actual.input_modes == expected.input_modes
        && actual.output_modes == expected.output_modes
        && actual.control_modes == expected.control_modes
        && actual.local_modes == expected.local_modes
        && actual.input_speed() == expected.input_speed()
        && actual.output_speed() == expected.output_speed()
        && [
            Code::VMIN,
            Code::VTIME,
            Code::VINTR,
            Code::VQUIT,
            Code::VSUSP,
        ]
        .into_iter()
        .all(|code| actual.special_codes[code] == expected.special_codes[code])
}
