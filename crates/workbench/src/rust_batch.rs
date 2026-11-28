//! 输入身份采用显式域和长度前缀，不把哈希当作新鲜度或权限证明。

use sha2::{Digest, Sha256};

use crate::RustSourceInput;

pub(crate) fn id(inputs: &[&RustSourceInput]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"studiolo-rust-call-links-v1\0");
    hash.update((inputs.len() as u64).to_be_bytes());
    for input in inputs {
        hash.update((input.path.len() as u64).to_be_bytes());
        hash.update(input.path.as_bytes());
        hash.update((input.content.len() as u64).to_be_bytes());
        hash.update(input.content.as_bytes());
    }
    studiolo_core::to_lower_hex(&hash.finalize())
}
