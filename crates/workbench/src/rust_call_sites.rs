//! 完整白名单输入的纯派生入口；任意失败都不返回部分源码证据。

use crate::{
    RustCallAnalysis, RustCallGraph, RustCallLinks, RustSourceError, RustSourceInput, rust_batch,
    rust_extract, rust_links, rust_walk, validate_rust_source_paths,
};

/// 从固定文件集合派生词法上下文和调用位置，不读取或执行项目内容。
pub fn derive_rust_call_sites(
    inputs: &[RustSourceInput],
) -> Result<RustCallGraph, RustSourceError> {
    derive(inputs, false).map(|analysis| analysis.graph)
}

/// 同一批 CST 同时产生原调用图和受限词法关联，不进行额外读取或解析。
pub fn derive_rust_call_analysis(
    inputs: &[RustSourceInput],
) -> Result<RustCallAnalysis, RustSourceError> {
    derive(inputs, true)
}

fn derive(
    inputs: &[RustSourceInput],
    with_links: bool,
) -> Result<RustCallAnalysis, RustSourceError> {
    if inputs.len() > 16 {
        return Err(RustSourceError::unsupported(
            "Rust 源码白名单超过 16 个文件。",
        ));
    }
    validate_rust_source_paths(
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
    let mut ordered = inputs.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.path.cmp(&right.path));
    let mut graph = RustCallGraph {
        files: Vec::new(),
        contexts: Vec::new(),
        calls: Vec::new(),
    };
    let mut links = RustCallLinks {
        version: 1,
        batch_id: if with_links {
            rust_batch::id(&ordered)
        } else {
            String::new()
        },
        associations: Vec::new(),
    };
    let mut nodes = 0;
    for (index, input) in ordered.iter().enumerate() {
        let tree = rust_walk::parse(&input.content)?;
        rust_walk::validate(&tree, &mut nodes)?;
        let first_call = graph.calls.len();
        rust_extract::extract(&tree, input, index, &mut graph)?;
        if with_links {
            links.associations.extend(rust_links::derive(
                &tree,
                input,
                index,
                &graph.calls[first_call..],
            )?);
        }
    }
    Ok(RustCallAnalysis { graph, links })
}
