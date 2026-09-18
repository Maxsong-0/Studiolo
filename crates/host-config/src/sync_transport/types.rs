//! 传输审批的只读事实；这些值是被批准的期望，不是运行时已核验的实际配置。

use std::path::{Path, PathBuf};

/// 传输私有根与镜像根的规范路径及真实对象身份。
///
/// `identity` 是观察时刻的 `(dev, ino)`；每次读取前后都必须重新比较，
/// 单次比较不能证明与后续 IO 的系统级原子性。
///
/// 这里的取值只是**可持久化的投影**，不是复验依据：Linux 把目录 rmdir 后同名 mkdir
/// 会发还同一个 inode 号，仅比较这两个数看不出根对象已被替换。真正的复验在
/// `ApprovedTransport::revalidate`，它对批准时持有的目录 FD 复查链接数与身份。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportRoots {
    private_root: PathBuf,
    private_identity: (u64, u64),
    mirror_root: PathBuf,
    mirror_identity: (u64, u64),
}

impl TransportRoots {
    pub(super) fn new(
        private_root: PathBuf,
        private_identity: (u64, u64),
        mirror_root: PathBuf,
        mirror_identity: (u64, u64),
    ) -> Self {
        Self {
            private_root,
            private_identity,
            mirror_root,
            mirror_identity,
        }
    }

    /// 只有 owner 写入的传输私有根；与 canonical、writer-data、审计、Bridge DB 两两不重叠。
    #[must_use]
    pub fn private_root(&self) -> &Path {
        &self.private_root
    }

    /// 传输私有根的 `(dev, ino)`。
    #[must_use]
    pub fn private_identity(&self) -> (u64, u64) {
        self.private_identity
    }

    /// Headless 专用镜像根。
    #[must_use]
    pub fn mirror_root(&self) -> &Path {
        &self.mirror_root
    }

    /// 镜像根的 `(dev, ino)`。
    #[must_use]
    pub fn mirror_identity(&self) -> (u64, u64) {
        self.mirror_identity
    }
}

/// 固定运行时：精确发布版本与只读安装闭包摘要；不保存任何凭证正文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportRuntime {
    node_path: PathBuf,
    cli_path: PathBuf,
    version: String,
    closure_digest: [u8; 32],
}

impl TransportRuntime {
    pub(super) fn new(
        node_path: PathBuf,
        cli_path: PathBuf,
        version: String,
        closure_digest: [u8; 32],
    ) -> Self {
        Self {
            node_path,
            cli_path,
            version,
            closure_digest,
        }
    }

    /// 固定 Node 可执行文件的绝对路径；运行时不做 PATH 搜索。
    #[must_use]
    pub fn node_path(&self) -> &Path {
        &self.node_path
    }

    /// 固定 `cli.js` 的绝对路径。
    #[must_use]
    pub fn cli_path(&self) -> &Path {
        &self.cli_path
    }

    /// 精确发布版本号；第一片固定 `0.0.14`。
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// 完整只读安装闭包摘要；不是单文件哈希，也不证明闭包已经实测。
    #[must_use]
    pub fn closure_digest(&self) -> [u8; 32] {
        self.closure_digest
    }
}

/// 管理员预置的受控 Linux 后端期望；批准本身不代表这些特权已经存在。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportBackend {
    service_unit: String,
    launcher_path: PathBuf,
    payload_cgroup_prefix: PathBuf,
    payload_uid: u32,
    payload_gid: u32,
}

impl TransportBackend {
    pub(super) fn new(
        service_unit: String,
        launcher_path: PathBuf,
        payload_cgroup_prefix: PathBuf,
        payload_uid: u32,
        payload_gid: u32,
    ) -> Self {
        Self {
            service_unit,
            launcher_path,
            payload_cgroup_prefix,
            payload_uid,
            payload_gid,
        }
    }

    /// 精确 systemd unit 名；owner 不接受任意 unit 或属性。
    #[must_use]
    pub fn service_unit(&self) -> &str {
        &self.service_unit
    }

    /// 可信 launcher 二进制的固定绝对路径。
    #[must_use]
    pub fn launcher_path(&self) -> &Path {
        &self.launcher_path
    }

    /// payload 子组的固定前缀；实际子组身份仍须由 launcher 返回并核验。
    #[must_use]
    pub fn payload_cgroup_prefix(&self) -> &Path {
        &self.payload_cgroup_prefix
    }

    /// 专用 payload UID；不得为 0，也不得等于 owner 自身身份。
    #[must_use]
    pub fn payload_uid(&self) -> u32 {
        self.payload_uid
    }

    /// 专用 payload GID。
    #[must_use]
    pub fn payload_gid(&self) -> u32 {
        self.payload_gid
    }
}

/// 运行预算；缺项或实际配置不支持则不得启动。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportBudget {
    /// wall deadline 秒数。
    pub deadline_seconds: u32,
    /// TERM grace 秒数。
    pub stop_grace_seconds: u32,
    /// stdout 字节限额。
    pub stdout_bytes: u32,
    /// stderr 字节限额。
    pub stderr_bytes: u32,
    /// 配置 JSON 字节上限；不超过 64 KiB。
    pub config_json_bytes: u32,
    /// 私有状态目录的磁盘 quota 字节数。
    pub private_state_bytes: u64,
    /// `sync.log` 的磁盘预算字节数；stdout 限额不覆盖它。
    pub log_bytes: u64,
    /// `memory.max` 期望字节数。
    pub memory_bytes: u64,
    /// `pids.max` 期望值。
    pub pids_max: u32,
    /// `cpu.max` 期望百分比；100 表示一个满核。
    pub cpu_max_percent: u32,
}
