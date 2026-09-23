//! 单调递增的 fencing 令牌 `Fence`：把一次尝试与更早的尝试隔离开。

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

/// 单调递增的令牌，用来把一次尝试与更早的尝试隔离开（fencing）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fence(NonZeroU64);

impl Fence {
    /// 构造非零的 fence；传入 0 时返回 `None`。
    #[must_use]
    pub fn new(value: u64) -> Option<Self> {
        NonZeroU64::new(value).map(Self)
    }

    /// 返回 fence 的数值。
    #[must_use]
    pub fn get(self) -> u64 {
        self.0.get()
    }
}
