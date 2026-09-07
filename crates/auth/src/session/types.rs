//! 会话公开 DTO 与失败模式；元信息永不携带凭证或凭证摘要。

use super::{CsrfToken, SessionToken};

/// 公开会话标识；v1 随机值或 v2 持久签发序号，不是凭证摘要。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionId(pub(super) String);

impl SessionId {
    /// 解析旧 32hex 或 `v2-` 加 16hex 正 i64 序号。
    pub fn parse(value: String) -> Result<Self, SessionError> {
        let legacy = value.len() == 32 && super::secrets::lower_hex(&value);
        let current = value
            .strip_prefix("v2-")
            .is_some_and(super::secrets::valid_sequence);
        if !legacy && !current {
            return Err(SessionError::Rejected);
        }
        Ok(Self(value))
    }

    /// 返回可公开的会话标识。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 会话状态；非 Active 一律不能认证。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// 已持久保存，但只有原签发调用在审计确认后才能激活。
    Pending,
    /// 已激活；每次使用仍须验证身份、时间和持久版本。
    Active,
    /// 已吊销，不能因审计补记或重新打开存储而恢复。
    Revoked,
    /// 活跃会话已到闲置或绝对期限，不再接受认证。
    Expired,
    /// 待激活会话已过绝对期限，不能再发布原凭证。
    Abandoned,
}

impl SessionState {
    pub(super) fn parse(value: &str) -> Result<Self, SessionError> {
        match value {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            "expired" => Ok(Self::Expired),
            "abandoned" => Ok(Self::Abandoned),
            _ => Err(SessionError::CorruptStore),
        }
    }
}

/// 有界会话列表中的非秘密元信息，不证明请求仍获授权。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    /// 独立生成的公开标识，不是令牌或其摘要。
    pub id: SessionId,
    /// 查询时观察到的持久状态，不是后续请求的授权证明。
    pub state: SessionState,
    /// 建立时间，以 UNIX 毫秒表示。
    pub created_at_ms: i64,
    /// 最近激活或成功检查的 UNIX 毫秒；待激活记录初值为建立时间。
    pub last_seen_at_ms: i64,
    /// 不随活动延长的绝对期限，以 UNIX 毫秒表示。
    pub absolute_expires_at_ms: i64,
}

/// 仅原签发调用成功时返回一次；秘密字段不实现 Serialize 或 Clone。
#[derive(Debug)]
pub struct IssuedSession {
    /// 本次成功激活时的非秘密元信息。
    pub session: SessionInfo,
    /// 仅供受信任传输层交付的会话秘密，不得写入日志或数据库。
    pub token: SessionToken,
    /// 与本会话独立生成并绑定的 CSRF 秘密，不得写入日志或数据库。
    pub csrf: CsrfToken,
}

/// 已持久吊销结果；审计暂不可用不撤销此结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revocation {
    /// 本次操作持久吊销的会话数量，不表示审计已全部送达。
    pub revoked: usize,
}

/// 固定错误，不输出底层 SQL、目录、身份内容、凭证或摘要。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// 主人绑定、期限或其他可信配置无效，或者与已有存储不一致。
    #[error("会话配置无效或与已有存储不一致")]
    Configuration,
    /// 不创建打开要求的固定数据库不存在；不会创建空库或自动迁移。
    #[error("会话数据库不存在，请先显式初始化或迁移")]
    MissingStore,
    /// 私有目录、文件权限、链接或对象身份不符合存储约束。
    #[error("会话存储目标不安全")]
    UnsafeStorage,
    /// 当前平台无法提供所需的私有存储保证。
    #[error("当前平台不支持会话私有存储")]
    UnsupportedPlatform,
    /// 数据库版本或应用标识未知，不能自动迁移或重建。
    #[error("会话存储格式不受支持")]
    UnsupportedSchema,
    /// 已验证的旧版存储需要受信任调用方显式迁移。
    #[error("会话存储需要显式迁移")]
    MigrationRequired,
    /// 已有存储的结构、数据或回执不符合完整性要求。
    #[error("会话存储完整性检查失败")]
    CorruptStore,
    /// 数据库读取、锁、事务或持久提交未成功。
    #[error("会话存储暂不可用")]
    StorageUnavailable,
    /// 权威审计写入或回执验证未成功，不得发布新凭证。
    #[error("会话审计暂不可用")]
    AuditUnavailable,
    /// 时钟无效、超出可表示范围或低于已持久观察的时间。
    #[error("会话时钟无效或发生回退")]
    ClockRollback,
    /// 身份、凭证、CSRF、会话状态或期限未通过检查。
    #[error("会话请求未通过验证")]
    Rejected,
    /// 安全随机源未能提供完整随机值。
    #[error("会话安全随机源暂不可用")]
    RandomUnavailable,
    /// 检测到随机标识或凭证重复，拒绝继续签发。
    #[error("会话随机值发生冲突")]
    RandomCollision,
    /// 有界存储或单调计数已达到上限，不能继续增加记录。
    #[error("会话存储容量已达上限")]
    Capacity,
    /// 账号或可信来源的持久验证预算已耗尽。
    #[error("验证次数已达上限，请等待当前限制窗口结束")]
    RateLimited,
    /// 尚未通过可信登记确认因子。
    #[error("尚未完成可信因子登记")]
    FactorUnavailable,
    /// 已登记的因子不能通过首次登记入口替换或清空。
    #[error("因子已经登记，不能重复登记或隐式替换")]
    AlreadyEnrolled,
    /// 仍有未完成或未确认交付的因子操作。
    #[error("已有因子验证正在处理，请等待完成或安全补记")]
    OperationPending,
    /// 当前会话没有仍有效的已确认近期认证。
    #[error("当前操作需要重新完成二次认证")]
    ReauthenticationRequired,
}
