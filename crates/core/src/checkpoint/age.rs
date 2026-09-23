//! 恢复点年龄：毫秒时间戳与按内容截止时刻计算的年龄。
//!
//! 时钟回退或时间证据缺失一律显示未知，不显示 0，也不显示绿色。

use serde::{Deserialize, Serialize};

/// 毫秒时间戳；只用于计算恢复点年龄，不引入时区语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstantMillis(i64);

impl InstantMillis {
    /// 从 Unix 毫秒构造。
    #[must_use]
    pub fn from_millis(value: i64) -> Self {
        Self(value)
    }

    /// 返回 Unix 毫秒。
    #[must_use]
    pub fn millis(self) -> i64 {
        self.0
    }
}

/// 恢复点年龄。时钟回退或时间证据缺失一律显示未知。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RecoveryPointAge {
    /// 已知年龄，从内容截止时刻起算。
    Known {
        /// 年龄毫秒数。
        millis: u64,
    },
    /// 未知年龄；不得显示为 0 或绿色。
    Unknown {
        /// 未知的具体原因。
        reason: AgeUnknownReason,
    },
}

/// 年龄未知的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgeUnknownReason {
    /// 当前时刻早于内容截止时刻，本地时钟发生回退。
    ClockWentBackwards,
    /// 缺少内容截止时刻等时间证据。
    MissingTimeEvidence,
}

/// 按内容截止时刻计算恢复点年龄；`now` 早于截止时刻即判为时钟回退。
#[must_use]
pub fn recovery_point_age(
    content_cutoff: Option<InstantMillis>,
    now: Option<InstantMillis>,
) -> RecoveryPointAge {
    let (Some(cutoff), Some(now)) = (content_cutoff, now) else {
        return RecoveryPointAge::Unknown {
            reason: AgeUnknownReason::MissingTimeEvidence,
        };
    };
    match now.millis().checked_sub(cutoff.millis()) {
        Some(delta) if delta >= 0 => RecoveryPointAge::Known {
            millis: delta.unsigned_abs(),
        },
        _ => RecoveryPointAge::Unknown {
            reason: AgeUnknownReason::ClockWentBackwards,
        },
    }
}
