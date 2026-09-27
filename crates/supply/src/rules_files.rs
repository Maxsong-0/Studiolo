//! 规则文件判定（ADR-026 D9／D10）：规则根名与「是否规则文件」只在这里定义一处。
//!
//! 规则文件＝直接位于 `_规则/` 下的 `.md` 文件，文件名即规则 subject；与是否已发布到策略平面无关。
//! `_规则/` 下的子目录（`references/`、`归档/`、`skills/` 等）不算规则文件，照项目白名单处理；
//! `项目/*/CLAUDE.md` 是项目文件（D11）。接入方的五条读取路径都经这一个判定裁决（D5）。

/// 规则根目录的逻辑路径（vault 根下）。
pub const RULES_ROOT: &str = "_规则";

/// `path` 是规则文件时返回其 subject（文件名），否则返回 `None`。
///
/// 只看逻辑路径的形状：`_规则/<名>`，`<名>` 不含 `/` 且扩展名为 `md`；不访问文件系统。
/// 扩展名不分大小写，与供给层判定 Markdown 的口径一致（`replica/verify.rs` 的 `markdown`）：
/// 否则 `_规则/X.MD` 会被当笔记供出却不算规则文件，绕过名单。这类名字过不了规则 subject
/// 的命名校验（只收小写 `.md`），永远不在名单里，因而一律拒绝（失败关闭）。
#[must_use]
pub fn rule_subject(path: &str) -> Option<&str> {
    let name = path.strip_prefix(RULES_ROOT)?.strip_prefix('/')?;
    let markdown = name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("md"));
    (!name.contains('/') && markdown).then_some(name)
}
