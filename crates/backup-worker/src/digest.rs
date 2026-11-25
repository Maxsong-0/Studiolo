//! SHA-256 摘要的文本写法只定在这一处：配置、固定包、restic 输出的快照号和读回核对都用它。

/// SHA-256 的文本写法：64 个小写十六进制字符。
pub(crate) fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 字节写成小写十六进制文本，每个字节两位。
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "../tests/support/digest.rs"]
mod tests;
