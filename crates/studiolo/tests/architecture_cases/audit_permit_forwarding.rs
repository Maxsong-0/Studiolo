//! 覆盖了完整操作许可的账本 Adapter，必须连同不等待的试取一起覆盖（BUG-050）。
//!
//! 看板租约巡检在备份冻结期间跳过这一轮，靠的是 `AuditLedger::try_enter_operation` 当场返回 `None`。
//! trait 的默认实现转 `enter_operation`：包装真实账本的 Adapter 只转发了 `enter_operation` 的话，试取会
//! 落回默认、冻结时等满 5 秒报错，巡检又会整个停掉、带着人侧退出（169 上 2026-11-23 那次）。
//!
//! 规则按记号判（注释、字符串不算）：`src/` 下每个 `impl … AuditLedger for …` 块里有
//! `fn enter_operation` 就必须有 `fn try_enter_operation`。不覆盖许可的 Adapter（内存、测试用）不管，
//! 默认实现对它们是对的。`src/` 下 `tests` 目录和 `*tests.rs` 里的测试夹具不算。

use crate::support::{
    crate_directories, is_test_fixture,
    rust_lexer::{block, lex},
    rust_sources, workspace_root,
};
use std::fs;

/// 覆盖了许可、却没覆盖试取的 `impl AuditLedger` 块数。
fn unforwarded(source: &str) -> usize {
    let tokens = lex(source);
    (0..tokens.len())
        .filter(|&index| tokens[index] == "impl" && is_ledger_impl(&tokens[index..]))
        .filter(|&index| {
            let body = block(&tokens[index..]);
            defines(body, "enter_operation") && !defines(body, "try_enter_operation")
        })
        .count()
}

/// `impl … {` 的头部里有 `AuditLedger for`（带不带路径前缀都算）。
fn is_ledger_impl(tokens: &[String]) -> bool {
    let header = tokens
        .iter()
        .take_while(|token| *token != "{" && *token != ";")
        .map(String::as_str)
        .collect::<Vec<_>>();
    header.windows(2).any(|pair| pair == ["AuditLedger", "for"])
}

/// 块体里定义了名叫 `name` 的方法。
fn defines(body: &[String], name: &str) -> bool {
    body.windows(2)
        .any(|pair| pair[0] == "fn" && pair[1] == name)
}

#[test]
fn every_ledger_adapter_that_overrides_the_permit_also_overrides_the_try() {
    let root = workspace_root();
    let mut violations = Vec::new();
    let mut overriding = 0;
    for directory in crate_directories(&root) {
        for path in rust_sources(&directory.join("src")) {
            let relative = path.strip_prefix(&root).expect("源码在工作区里");
            if is_test_fixture(relative) {
                continue;
            }
            let source = fs::read_to_string(&path).expect("源码必须可读");
            if source.contains("fn enter_operation") {
                overriding += 1;
            }
            if unforwarded(&source) > 0 {
                violations.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "这些账本 Adapter 覆盖了 enter_operation 却没覆盖 try_enter_operation，冻结时试取会等满 5 秒报错：{violations:?}"
    );
    // 真账本、看板的包装层、worker 的跨进程客户端至少这三处在扫描范围里；扫不到说明遍历坏了。
    assert!(overriding >= 3, "只扫到 {overriding} 个覆盖许可的文件");
}

#[test]
fn the_forwarding_rule_flags_only_a_wrapper_that_forwards_the_permit_alone() {
    let wrapper = "impl AuditLedger for Shared {
        fn enter_operation(&self) -> R { self.0.enter_operation() }
        fn append(&self, event: AuditEvent) -> A { self.0.append(event) }
    }";
    assert_eq!(unforwarded(wrapper), 1);
    let both = wrapper.replace(
        "fn append",
        "fn try_enter_operation(&self) -> T { self.0.try_enter_operation() } fn append",
    );
    assert_eq!(unforwarded(&both), 0);
    // 带路径前缀、带生命周期的写法也认；注释和字符串里的方法名不算数。
    let prefixed = wrapper.replace(
        "impl AuditLedger for Shared",
        "impl studiolo_audit_ledger::AuditLedger for Client<'_>",
    );
    assert_eq!(unforwarded(&prefixed), 1);
    let commented = wrapper.replace(
        "fn append",
        "// fn try_enter_operation\n fn note() -> &'static str { \"fn try_enter_operation\" } fn append",
    );
    assert_eq!(unforwarded(&commented), 1);
    // 不覆盖许可的（内存、测试 Adapter）和别的 trait 的 impl 不管。
    assert_eq!(
        unforwarded("impl AuditLedger for Memory { fn append(&self) {} }"),
        0
    );
    assert_eq!(
        unforwarded("impl Other for Shared { fn enter_operation(&self) {} }"),
        0
    );
    // 一个文件里两个包装层，各算各的。
    assert_eq!(unforwarded(&format!("{wrapper}\n{wrapper}\n{both}")), 2);
}
