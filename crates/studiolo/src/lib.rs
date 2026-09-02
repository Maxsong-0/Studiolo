//! Studiolo：自托管的知识与项目工作台。
//!
//! 占位 crate，用于保留 crates.io 名称。实现代码在 M0 落地。

/// 返回 crate 版本，占位用。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_not_empty() {
        assert!(!super::version().is_empty());
    }
}
