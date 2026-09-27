//! 内容修订值 `Revision`：绑定 Vault 身份与逻辑路径的 SHA-256 摘要。

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{RevisionHasher, VaultPath, VaultUuid};

/// 绑定到 Vault 身份与逻辑路径的内容修订值（Revision）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
// 摘要字段放宽到 crate 内可见：同级的 `revision_hasher` 模块在 `finish` 里直接构造 Revision。
pub struct Revision(pub(crate) [u8; 32]);

impl Revision {
    /// 计算 Vault 中某路径上内容的 SHA-256 修订值。
    #[must_use]
    pub fn compute(vault: VaultUuid, path: &VaultPath, content: &[u8]) -> Self {
        let mut hasher = RevisionHasher::new(vault, path);
        hasher.update(content);
        hasher.finish()
    }

    /// 由 32 字节摘要原值构造。`Revision` 是比较用的修订值而不是能力：`Deserialize` 本就接受任意字节，
    /// 这里不放宽任何边界。Bridge 证据（`studiolo-sync-bridge` 的 `FileVersion::revision`）直接承载这
    /// 32 字节，条件写／条件删除的前置因此在任何一轮都能还原。
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// 摘要的 32 字节原值；与 [`Self::from_bytes`] 互逆。
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    /// 返回小写十六进制摘要。
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut output = String::with_capacity(64);
        for byte in self.0 {
            use fmt::Write as _;
            write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
        }
        output
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}
