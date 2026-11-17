//! JSON 输出有界保存，文件复验只流式保留摘要，stderr 只计数；三者共用管道、期限和清理。
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub(super) trait Sink {
    fn limit(&self) -> u64;
    fn append(&mut self, bytes: &[u8]);
}
pub(super) struct Buffered {
    pub bytes: Zeroizing<Vec<u8>>,
    limit: usize,
}
impl Buffered {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Zeroizing::new(Vec::with_capacity(limit)),
            limit,
        }
    }
}
impl Sink for Buffered {
    fn limit(&self) -> u64 {
        self.limit as u64
    }
    fn append(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
}
/// stderr 只计数、不留内容：内容从来不读，上限只用来界定输出量。
pub(super) struct Discarded {
    limit: u64,
}
impl Discarded {
    pub fn new(limit: u64) -> Self {
        Self { limit }
    }
}
impl Sink for Discarded {
    fn limit(&self) -> u64 {
        self.limit
    }
    fn append(&mut self, _: &[u8]) {}
}
pub(super) struct Hashed {
    pub digest: Sha256,
    limit: u64,
}
impl Hashed {
    pub fn new(limit: u64) -> Self {
        Self {
            digest: Sha256::new(),
            limit,
        }
    }
}
impl Sink for Hashed {
    fn limit(&self) -> u64 {
        self.limit
    }
    fn append(&mut self, bytes: &[u8]) {
        self.digest.update(bytes);
    }
}
