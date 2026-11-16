use super::StoreError;
use std::io::Write;

/// 在序列化期间截断超额输入，避免先分配无限 Vec 再检查大小。
pub(super) fn encode(value: &impl serde::Serialize, max: u64) -> Result<Vec<u8>, StoreError> {
    let mut output = Limited {
        bytes: Vec::new(),
        max,
        exceeded: false,
    };
    serde_json::to_writer(&mut output, value).map_err(|_| {
        if output.exceeded {
            StoreError::LimitExceeded
        } else {
            StoreError::CorruptStore
        }
    })?;
    Ok(output.bytes)
}
struct Limited {
    bytes: Vec<u8>,
    max: u64,
    exceeded: bool,
}
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if (bytes.len() as u64) > self.max.saturating_sub(self.bytes.len() as u64) {
            self.exceeded = true;
            return Err(std::io::Error::other("备份状态超过字节上限"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
