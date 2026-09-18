//! 备份审批的封闭参数类型；只有固定包名、数字与变量名，不含地址或凭据。

/// 备份目标的形态。配置只声明形态，不声明「已经具备」什么能力。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupTargetShape {
    /// 本机临时仓库；只用于夹具与负测，不构成灾难恢复目标。
    LocalTemporaryRepository,
    /// 普通 SMB/NFS 目录挂载；本身不提供不可变保护。
    PlainSmbOrNfsDirectory,
    /// 专用的仅追加／不可变目标。
    DedicatedAppendOnlyTarget,
}

impl BackupTargetShape {
    /// 该形态在配置层面是否**可能**声明不可变保护。
    ///
    /// 只有专用目标可以声明；即便如此，声明也只是待核验的主张。
    #[must_use]
    pub fn may_claim_immutability(self) -> bool {
        matches!(self, Self::DedicatedAppendOnlyTarget)
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "local_temporary_repository" => Some(Self::LocalTemporaryRepository),
            "plain_smb_or_nfs_directory" => Some(Self::PlainSmbOrNfsDirectory),
            "dedicated_append_only_target" => Some(Self::DedicatedAppendOnlyTarget),
            _ => None,
        }
    }
}

/// 固定 restic 包；换版本必须改配置并重新验收。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResticPin {
    pub(super) package: String,
    pub(super) version: String,
}

impl ResticPin {
    /// 固定包名。
    #[must_use]
    pub fn package(&self) -> &str {
        &self.package
    }

    /// 固定版本号。
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// 凭据引用：只保留环境变量名，永远不保留值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialReference {
    pub(super) logical_name: String,
    pub(super) environment_variable: String,
}

impl CredentialReference {
    /// 逻辑名，例如 `restic_repository_password`。
    #[must_use]
    pub fn logical_name(&self) -> &str {
        &self.logical_name
    }

    /// 去哪拿：环境变量名。
    #[must_use]
    pub fn environment_variable(&self) -> &str {
        &self.environment_variable
    }
}

/// 调度与完成预算，单位毫秒。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduleBudget {
    pub(super) interval_millis: u64,
    pub(super) completion_millis: u64,
    pub(super) rpo_millis: u64,
}

impl ScheduleBudget {
    /// 调度间隔毫秒数。
    #[must_use]
    pub fn interval_millis(self) -> u64 {
        self.interval_millis
    }

    /// 单次完成预算毫秒数。
    #[must_use]
    pub fn completion_millis(self) -> u64 {
        self.completion_millis
    }

    /// 目标 RPO 毫秒数。
    #[must_use]
    pub fn rpo_millis(self) -> u64 {
        self.rpo_millis
    }

    /// 间隔加完成预算不得超过 RPO；相加溢出一律判为不满足。
    #[must_use]
    pub fn satisfies_rpo(self) -> bool {
        self.interval_millis
            .checked_add(self.completion_millis)
            .is_some_and(|total| total <= self.rpo_millis)
    }
}

/// 保留策略；清理使用独立身份，不与追加凭据共用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub(super) keep_last: u32,
    pub(super) keep_daily: u32,
}

impl RetentionPolicy {
    /// 保留最近多少个恢复点。
    #[must_use]
    pub fn keep_last(self) -> u32 {
        self.keep_last
    }

    /// 保留多少个每日恢复点。
    #[must_use]
    pub fn keep_daily(self) -> u32 {
        self.keep_daily
    }
}
