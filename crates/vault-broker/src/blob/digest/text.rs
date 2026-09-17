//! 完整 UTF-8 字节事实，最多暂存跨块的三个未完字节；不判断 Markdown 内容策略。
#[derive(Default)]
pub(super) struct Utf8 {
    pending: [u8; 4],
    len: usize,
    invalid: bool,
}
impl Utf8 {
    pub(super) fn update(&mut self, mut bytes: &[u8]) {
        if self.invalid {
            return;
        }
        if self.len != 0 {
            let need = match self.pending[0] {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => {
                    self.invalid = true;
                    return;
                }
            };
            let count = (need - self.len).min(bytes.len());
            self.pending[self.len..self.len + count].copy_from_slice(&bytes[..count]);
            self.len += count;
            bytes = &bytes[count..];
            if self.len < need {
                return;
            }
            if std::str::from_utf8(&self.pending[..need]).is_err() {
                self.invalid = true;
                return;
            }
            self.len = 0;
        }
        if let Err(error) = std::str::from_utf8(bytes) {
            if error.error_len().is_some() {
                self.invalid = true;
                return;
            }
            let tail = &bytes[error.valid_up_to()..];
            self.pending[..tail.len()].copy_from_slice(tail);
            self.len = tail.len();
        }
    }
    pub(super) fn complete(&self) -> bool {
        !self.invalid && self.len == 0
    }
}
