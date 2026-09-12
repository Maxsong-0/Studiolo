//! 完整白名单输入的纯派生入口；任意失败都不返回部分源码证据。

use std::collections::BTreeMap;

use crate::{
    RustCallGraph, RustSourceError, RustSourceInput, rust_extract, rust_walk,
    validate_rust_source_paths,
};

/// 从固定文件集合派生词法上下文和调用位置，不读取或执行项目内容。
pub fn derive_rust_call_sites(
    inputs: &[RustSourceInput],
) -> Result<RustCallGraph, RustSourceError> {
    if inputs.len() > 16 {
        return Err(RustSourceError::unsupported(
            "Rust 源码白名单超过 16 个文件。",
        ));
    }
    let paths = validate_rust_source_paths(
        &inputs
            .iter()
            .map(|item| item.path.clone())
            .collect::<Vec<_>>(),
    )?;
    let mut total_bytes = 0_usize;
    for item in inputs {
        if item.content.len() > 64 * 1024 {
            return Err(RustSourceError::unsupported(
                "Rust 源码超过单文件 64 KiB 预算。",
            ));
        }
        total_bytes += item.content.len();
        if total_bytes > 512 * 1024 {
            return Err(RustSourceError::unsupported(
                "Rust 源码超过整批 512 KiB 预算。",
            ));
        }
    }
    let ordered = inputs
        .iter()
        .map(|item| (item.path.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    let mut graph = RustCallGraph {
        files: Vec::new(),
        contexts: Vec::new(),
        calls: Vec::new(),
    };
    let mut nodes = 0;
    for (index, path) in paths.iter().enumerate() {
        let input = ordered[path.as_str()];
        let tree = rust_walk::parse(&input.content)?;
        rust_walk::validate(&tree, &mut nodes)?;
        rust_extract::extract(&tree, input, index, &mut graph)?;
    }
    Ok(graph)
}
