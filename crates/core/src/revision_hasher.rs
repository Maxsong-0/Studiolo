//! 内容 Revision 的唯一增量实现；分块边界不进入摘要协议。
use crate::{Revision, VaultPath, VaultUuid};
use sha2::{Digest, Sha256};

/// 与 [`Revision::compute`] 完全等价的增量路径绑定哈希，不保存正文。
pub struct RevisionHasher {
    hasher: Sha256,
}
impl RevisionHasher {
    /// 固定原有 v1 域、Vault UUID 显示编码和路径 UTF-8 编码。
    #[must_use]
    pub fn new(vault: VaultUuid, path: &VaultPath) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"studiolo-revision-v1\0");
        hasher.update(vault.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(path.as_str().as_bytes());
        hasher.update(b"\0");
        Self { hasher }
    }
    /// 追加原始字节；空块合法，不附加长度、分隔符或块编号。
    pub fn update(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
    }
    /// 消费哈希器并生成 Revision；未追加内容时得到空文件的路径绑定摘要。
    #[must_use]
    pub fn finish(self) -> Revision {
        Revision(self.hasher.finalize().into())
    }
}
