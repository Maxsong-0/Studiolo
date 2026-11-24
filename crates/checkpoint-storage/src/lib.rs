//! 只供持有固定数据库的存储 owner 使用；协调器不得依赖本 crate 或获得 Connection。
//! 本库只保证单库冻结与导出，完整语义操作、pending 和 ACK 必须由 owner 另行排空核验。
mod digest;
mod export;
mod flush;
mod paths;
mod sqlite;
use rusqlite::Connection;
use std::{path::Path, sync::Arc, time::Instant};
use studiolo_core::checkpoint::WalCheckpointProof;

/// 允许封存的固定库名；不接受任意目标文件名。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseFile {
    /// 人侧认证库。
    Auth,
    /// 一个明确绑定的审计账本。
    Audit,
    /// TaskEngine 控制库。
    Control,
    /// 双向同步状态。
    Bridge,
    /// 接入方身份与索引。
    Agent,
}
impl DatabaseFile {
    /// 固定文件名。
    pub fn name(self) -> &'static str {
        match self {
            Self::Auth => "auth.sqlite3",
            Self::Audit => "audit.sqlite3",
            Self::Control => "control.sqlite3",
            Self::Bridge => "bridge.sqlite3",
            Self::Agent => "agent.sqlite3",
        }
    }
}

/// 不带源路径、SQL 或秘密的封闭失败类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// 原截止时间已到，未获得完整证明。
    Deadline,
    /// 身份、路径、权限或目标复用不合法。
    UnsafeStorage,
    /// schema、内容摘要或 owner 业务校验失败。
    Invalid,
    /// 存储或 SQLite 不可用。
    Unavailable,
}
impl std::fmt::Display for Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(match self {
            Self::Deadline => "封存超过截止时间",
            Self::UnsafeStorage => "封存存储身份不合法",
            Self::Invalid => "封存核验失败",
            Self::Unavailable => "封存存储不可用",
        })
    }
}
impl std::error::Error for Error {}

/// 绑定原目录与原数据库 inode；只能由实际 owner 从其当前 Storage 构造。
#[derive(Clone)]
pub struct Source {
    binding: Arc<paths::Binding>,
}
impl Source {
    /// Control 启动先固定由可信装配创建的空文件，再初始化同一 inode；不创建文件。
    /// 此能力本身不是封存证明，freeze 后仍须通过 Control 的完整 schema 校验。
    pub fn pin_control_before_initialization(root: &Path) -> Result<Self, Error> {
        Ok(Self {
            binding: Arc::new(paths::Binding::control_before_initialization(root)?),
        })
    }
    /// 只绑定已存在的0700私有目录和0600固定库；不创建、不迁移源。
    pub fn bind(root: &Path, file: DatabaseFile) -> Result<Self, Error> {
        Ok(Self {
            binding: Arc::new(paths::Binding::source(root, file)?),
        })
    }
    /// 独立连接持有 RESERVED 写锁，WAL 整个回写主库之后，在另一只读连接上开读事务把库定在这一刻；
    /// backup 从这个定格的读连接做，不在写事务上执行。
    /// 调用者须先取得完整语义操作屏障；冻结本身不能证明跨库已排空。
    pub fn freeze(&self, deadline: Instant) -> Result<FrozenSqlite, Error> {
        sqlite::freeze(self.binding.clone(), deadline)
    }
    /// 复核源仍是绑定时的目录与数据库。
    pub fn check(&self) -> Result<(), Error> {
        self.binding.check()
    }
    /// 比较两份能力是否固定同一原目录和原库；任一来源已改变时拒绝，而非比较路径字符串。
    pub fn same_origin(&self, other: &Self) -> Result<bool, Error> {
        self.binding.same_origin(&other.binding)
    }
}

/// 一个单库窗口：先持有 RESERVED 写锁（挡住别的写者），同时一条只读连接开着读事务，把库定在冻结那一刻。
/// [`FrozenSqlite::release`] 放开写锁之后，定格的读连接还在，导出读到的仍是冻结那一刻（BUG-045：
/// 拷贝和校验挪到放开之后做，冻结窗口里只定格）。丢弃会关闭连接并回滚，不遗留持久锁。
/// 类型不 Clone、不暴露写锁连接；owner 必须把它保留到导出结束。
pub struct FrozenSqlite {
    // 先关闭连接再关闭身份 pin。放开之后写锁连接是 None。
    reserve: Option<Connection>,
    source: Connection,
    binding: Arc<paths::Binding>,
    deadline: Instant,
    wal: WalCheckpointProof,
}
impl FrozenSqlite {
    /// 由 owner 读取固定 schema、pending、业务水位及原 ACK；连接仅活在回调中。
    /// 放开写锁之后读到的仍是冻结那一刻。
    pub fn inspect<T>(
        &self,
        inspect: impl FnOnce(&Connection) -> Result<T, Error>,
    ) -> Result<T, Error> {
        self.check()?;
        let value =
            inspect(&self.source).map_err(|error| sqlite::deadline_error(error, self.deadline))?;
        self.check()?;
        Ok(value)
    }
    /// 放开写锁，别的写者从此可以写；定格的读连接留着，之后的读取和导出用新的截止时间。
    /// 只能放开一次；调用者须同时放开跨库的语义屏障，二者都由组合根按逆序做。
    pub fn release(&mut self, deadline: Instant) -> Result<(), Error> {
        let reserve = self.reserve.take().ok_or(Error::Invalid)?;
        drop(reserve);
        self.deadline = deadline;
        sqlite::limit(&self.source, deadline)?;
        self.check()
    }
    /// 导出到新空0700目录。读的是冻结那一刻的定格；放开写锁前后都可以调。
    /// 副本经 owner 校验及完整内容摘要核对（和定格的源逐表比）；WAL 证明是冻结时取的。
    /// 目标残片不覆盖、不清理；只有返回成功时才可加入封存清单。
    pub fn export<T: Eq>(
        &self,
        target: &Path,
        expected: &T,
        inspect: impl Fn(&Connection) -> Result<T, Error>,
    ) -> Result<WalCheckpointProof, Error> {
        export::export(self, target, expected, inspect)
    }
    fn check(&self) -> Result<(), Error> {
        sqlite::check_deadline(self.deadline)?;
        // 读事务没了就不再是定格：任何一次读都可能读到冻结之后的内容。
        if self.source.is_autocommit()
            || self
                .reserve
                .as_ref()
                .is_some_and(|reserve| reserve.is_autocommit())
        {
            return Err(Error::Invalid);
        }
        self.binding.check()
    }
}
