use super::FINAL_MANIFEST_MAX_BYTES;
use serde::Serialize;
use std::io::Write;

/// 最终记录只返回固定失败分类，不带路径、正文或秘密。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinalManifestError {
    /// 作业/引用/摘要/时序或 schema 不自洽。
    Invalid,
    /// 非正文最终记录超过固定预算。
    TooLarge,
}
impl std::fmt::Display for FinalManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "最终备份记录无效：{self:?}")
    }
}
impl std::error::Error for FinalManifestError {}
pub(super) fn encode(value: &impl Serialize) -> Result<Vec<u8>, FinalManifestError> {
    let mut output = Limited(Vec::new());
    serde_json::to_writer(&mut output, value).map_err(|_| FinalManifestError::TooLarge)?;
    Ok(output.0)
}
struct Limited(Vec<u8>);
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > FINAL_MANIFEST_MAX_BYTES - self.0.len() {
            return Err(std::io::Error::other("最终记录超过预算"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn digest(bytes: &[u8]) -> String {
    studiolo_core::sha256_hex(bytes)
}
pub(super) fn digest_shaped(value: &str) -> bool {
    studiolo_core::is_sha256_hex(value)
}
