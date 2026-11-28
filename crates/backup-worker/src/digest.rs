//! SHA-256 摘要的文本写法：配置、固定包、restic 输出的快照号和读回核对都用它；定义在 `studiolo-core`（T-246），
//! 两侧用例在 `crates/core/tests/hex_contract.rs`。

pub(crate) use studiolo_core::{is_sha256_hex, to_lower_hex as hex};
