//! 白名单只接受规范可见的 Rust 相对路径，验证完成后才允许采集。

use std::collections::{BTreeMap, BTreeSet};

use studiolo_core::VaultPath;

use crate::RustSourceError;

/// 原子验证完整白名单，返回唯一且稳定排序的路径。
pub fn validate_rust_source_paths(paths: &[String]) -> Result<Vec<String>, RustSourceError> {
    if paths.is_empty() {
        return Err(RustSourceError::invalid("Rust 源码白名单不可为空。"));
    }
    if paths.len() > 16 {
        return Err(RustSourceError::unsupported(
            "Rust 源码白名单超过 16 个文件。",
        ));
    }
    let mut ordered = BTreeSet::new();
    let mut prefixes = BTreeMap::new();
    for path in paths {
        if path.len() > 1024 || path.split('/').count() > 16 {
            return Err(RustSourceError::unsupported(
                "Rust 源码路径超过 1024 字节或 16 分量。",
            ));
        }
        if !path.ends_with(".rs")
            || path.chars().any(invisible)
            || VaultPath::parse(path).is_err()
            || path.split('/').any(|part| part.starts_with('.'))
            || path.as_bytes().get(1) == Some(&b':')
        {
            return Err(RustSourceError::invalid(
                "Rust 源码白名单包含不规范、隐藏或非 .rs 路径。",
            ));
        }
        if !ordered.insert(path.clone()) {
            return Err(RustSourceError::invalid("Rust 源码白名单包含重复路径。"));
        }
        let mut prefix = String::new();
        for part in path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if let Some(previous) = prefixes.insert(prefix.to_lowercase(), prefix.clone())
                && previous != prefix
            {
                return Err(RustSourceError::invalid(
                    "Rust 源码白名单包含大小写路径前缀碰撞。",
                ));
            }
        }
    }
    // 一个文件不能同时充当另一白名单文件的父目录。
    for path in &ordered {
        for (index, _) in path.match_indices('/') {
            if ordered.contains(&path[..index]) {
                return Err(RustSourceError::invalid(
                    "Rust 源码白名单混用了文件和目录身份。",
                ));
            }
        }
    }
    Ok(ordered.into_iter().collect())
}

/// Unicode 控制、格式字符和行/段分隔符均不能充当可见名称。
pub(crate) fn invisible(value: char) -> bool {
    value.is_control()
        || matches!(value,
        '\u{00ad}' | '\u{0600}'..='\u{0605}' | '\u{061c}' | '\u{06dd}' | '\u{070f}' |
        '\u{0890}'..='\u{0891}' | '\u{08e2}' | '\u{180e}' | '\u{200b}'..='\u{200f}' |
        '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}' |
        '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{110bd}' | '\u{110cd}' |
        '\u{13430}'..='\u{1343f}' | '\u{1bca0}'..='\u{1bca3}' |
        '\u{1d173}'..='\u{1d17a}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}')
}
