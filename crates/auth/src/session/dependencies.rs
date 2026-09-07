//! 时间与随机源可替换；生产实现不接受请求提供的时间或种子。

use super::SessionError;

/// 可信程序装配的时间 Adapter；请求正文和转发头不能提供当前时间。
///
/// 返回值仍由会话 Module 与持久时间地板比较，替换 Adapter 不能绕过回退检查。
pub trait SessionClock: Send + Sync {
    /// 返回 UNIX 毫秒；无法取得可表示时间时返回错误，不能猜测或沿用旧值。
    fn now_unix_ms(&self) -> Result<i64, SessionError>;
}

/// 使用操作系统墙上时钟的生产 Adapter，不自行消除或掩盖时钟回退。
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemSessionClock;

impl SessionClock for SystemSessionClock {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        let duration = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| SessionError::ClockRollback)?;
        i64::try_from(duration.as_millis()).map_err(|_| SessionError::ClockRollback)
    }
}

/// 可信程序装配的随机 Adapter；生产实现必须提供密码学安全随机字节。
///
/// 测试可装配确定性实现；不得将请求提供的种子或普通伪随机源接入生产。
pub trait SessionRandom: Send + Sync {
    /// 完整填充目标缓冲区；失败时调用者不得使用可能仅部分填充的数据。
    fn fill(&self, destination: &mut [u8]) -> Result<(), SessionError>;
}

/// 通过操作系统安全随机源填充缓冲区的生产 Adapter。
#[derive(Clone, Copy, Debug, Default)]
pub struct OsSessionRandom;

impl SessionRandom for OsSessionRandom {
    fn fill(&self, destination: &mut [u8]) -> Result<(), SessionError> {
        getrandom::fill(destination).map_err(|_| SessionError::RandomUnavailable)
    }
}
