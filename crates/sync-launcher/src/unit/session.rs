//! session 目录与控制 FIFO 的真实 IO（守卫按精确记号登记）：目录只在 sessions 父目录之下建，
//! FIFO 写端由 launcher 以读写方式持有——写入永不阻塞、无读者也不失败，丢弃写端就是关门。

use super::reads::stat;
use crate::protocol::Verdict;
use rustix::fs::{CWD, Mode};
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// 一次 launch 的 session 目录：控制 FIFO、stdout、stderr；目录 0700（进程 umask 077）。
pub struct SessionDir {
    path: PathBuf,
}

impl SessionDir {
    /// 新建目录与控制 FIFO，并以读写方式持有 FIFO 写端。
    ///
    /// # Errors
    /// 建目录、建 FIFO 或打开失败返回 `ManagerUnavailable`（目录会被清掉）。
    pub fn create(sessions: &Path, hex: &str) -> Result<(Self, ReleaseGate), Verdict> {
        let path = sessions.join(hex);
        std::fs::create_dir(&path).map_err(|_| Verdict::ManagerUnavailable)?;
        let dir = Self { path };
        let fifo = dir.release_fifo();
        let opened = rustix::fs::mkfifoat(CWD, &fifo, Mode::from_raw_mode(0o600))
            .map_err(io::Error::from)
            .and_then(|()| {
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&fifo)
            });
        match opened {
            Ok(file) => Ok((dir, ReleaseGate(file))),
            Err(_) => {
                dir.remove();
                Err(Verdict::ManagerUnavailable)
            }
        }
    }

    /// 接管已存在的 session 目录（launcher 重启后）。
    #[must_use]
    pub fn open(sessions: &Path, hex: &str) -> Option<Self> {
        let path = sessions.join(hex);
        stat(&path).map(|_| Self { path })
    }

    /// 删除整个 session 目录。
    pub fn remove(&self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }

    /// 控制 FIFO（`StandardInput=file:`）。
    #[must_use]
    pub fn release_fifo(&self) -> PathBuf {
        self.path.join("release.fifo")
    }

    /// payload 的 stdout 文件（头部是垫片的报告帧）。
    #[must_use]
    pub fn stdout(&self) -> PathBuf {
        self.path.join("stdout")
    }

    /// payload 的 stderr 文件。
    #[must_use]
    pub fn stderr(&self) -> PathBuf {
        self.path.join("stderr")
    }
}

/// 控制 FIFO 的写端：先写启动事实帧，放行时写一字节；丢弃它就是关门（垫片读到 EOF 即退出 4）。
pub struct ReleaseGate(File);

impl ReleaseGate {
    /// 写入管道。
    ///
    /// # Errors
    /// 写失败返回 `ManagerUnavailable`。
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), Verdict> {
        self.0
            .write_all(bytes)
            .map_err(|_| Verdict::ManagerUnavailable)
    }
}
