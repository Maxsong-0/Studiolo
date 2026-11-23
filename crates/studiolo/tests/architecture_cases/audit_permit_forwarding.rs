//! 覆盖了完整操作许可的账本 Adapter，必须连同不等待的试取一起覆盖，而且试取真的不等（BUG-050）。
//!
//! 看板租约巡检在备份冻结期间跳过这一轮，靠的是 `AuditLedger::try_enter_operation` 当场返回 `None`。
//! trait 的默认实现转 `enter_operation`：包装真实账本的 Adapter 只转发了 `enter_operation` 的话，试取会
//! 落回默认、冻结时等满 5 秒报错，巡检又会整个停掉、带着人侧退出（169 上 2026-11-23 那次）。
//! 覆盖了试取、方法体里却转去调阻塞的 `enter_operation`，是同一个症状。
//!
//! 规则按记号判（注释、字符串不算）：`src/` 下每个 `impl … AuditLedger for …` 块里有
//! `fn enter_operation` 就必须有 `fn try_enter_operation`，它的方法体里不许出现 `enter_operation`，
//! 并且要么转发给不等待的试取（`try_enter_operation`、真账本的 `try_operation_permit`），要么显式返回 `Err`。
//! 不覆盖许可的 Adapter（内存、测试用）不管，默认实现对它们是对的。`src/` 下 `tests` 目录和 `*tests.rs` 里的测试夹具不算。

use crate::support::{
    crate_directories, is_test_fixture,
    rust_lexer::{block, lex},
    rust_sources, workspace_root,
};
use std::fs;

/// 试取的方法体里出现这些记号之一，才算转发给了不等待的试取或显式拒绝。
const NON_BLOCKING: [&str; 3] = ["try_enter_operation", "try_operation_permit", "Err"];
/// 至少这几处覆盖了许可的 Adapter 在扫描范围里：真账本、看板的包装层、worker 的跨进程客户端。
const KNOWN_ADAPTERS: [&str; 3] = [
    "crates/audit-ledger/src/sqlite.rs",
    "crates/local-server/src/board/mod.rs",
    "crates/backup-worker/src/audit.rs",
];

/// 一个文件里覆盖了许可的 `impl AuditLedger` 块：（块数，其中试取没覆盖或会等的块数）。
fn ledger_impls(source: &str) -> (usize, usize) {
    let tokens = lex(source);
    let overriding = (0..tokens.len())
        .filter(|&index| tokens[index] == "impl" && is_ledger_impl(&tokens[index..]))
        .map(|index| block(&tokens[index..]))
        .filter(|body| method(body, "enter_operation").is_some())
        .collect::<Vec<_>>();
    let unforwarded = overriding
        .iter()
        .filter(|body| match method(body, "try_enter_operation") {
            None => true,
            Some(try_body) => {
                try_body.iter().any(|token| token == "enter_operation")
                    || !try_body
                        .iter()
                        .any(|token| NON_BLOCKING.contains(&token.as_str()))
            }
        })
        .count();
    (overriding.len(), unforwarded)
}

/// 覆盖了许可、却没覆盖试取或试取会等的 `impl AuditLedger` 块数。
fn unforwarded(source: &str) -> usize {
    ledger_impls(source).1
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

/// 块体里名叫 `name` 的方法的方法体；没定义这个方法就是 `None`。
fn method<'a>(body: &'a [String], name: &str) -> Option<&'a [String]> {
    body.windows(2)
        .position(|pair| pair[0] == "fn" && pair[1] == name)
        .map(|index| block(&body[index..]))
}

#[test]
fn every_ledger_adapter_that_overrides_the_permit_also_overrides_the_try() {
    let root = workspace_root();
    let mut violations = Vec::new();
    let mut overriding = Vec::new();
    for directory in crate_directories(&root) {
        for path in rust_sources(&directory.join("src")) {
            let relative = path.strip_prefix(&root).expect("源码在工作区里");
            if is_test_fixture(relative) {
                continue;
            }
            let source = fs::read_to_string(&path).expect("源码必须可读");
            let relative = relative.to_string_lossy().replace('\\', "/");
            let (impls, unforwarded) = ledger_impls(&source);
            if impls > 0 {
                overriding.push(relative.clone());
            }
            if unforwarded > 0 {
                violations.push(relative);
            }
        }
    }
    assert!(
        violations.is_empty(),
        "这些账本 Adapter 覆盖了 enter_operation，却没覆盖 try_enter_operation 或试取里会等（转调 enter_operation、\
         没转发给 try_enter_operation／try_operation_permit 也没显式返回 Err），冻结时试取会等满 5 秒报错：{violations:?}"
    );
    // 扫不到这几处说明遍历坏了，守卫形同虚设。
    for known in KNOWN_ADAPTERS {
        assert!(
            overriding.iter().any(|path| path == known),
            "没扫到覆盖许可的 Adapter {known}；扫到的：{overriding:?}"
        );
    }
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

#[test]
fn a_try_that_still_waits_is_flagged_and_real_shapes_pass() {
    let with_try = |try_body: &str| {
        format!(
            "impl AuditLedger for Shared {{
                fn enter_operation(&self) -> R {{ self.0.enter_operation() }}
                fn try_enter_operation(&self) -> T {{ {try_body} }}
            }}"
        )
    };
    // 试取里转调阻塞的许可、或者永远说「冻结中」：都不算转发。
    assert_eq!(
        unforwarded(&with_try("self.0.enter_operation().map(Some)")),
        1
    );
    assert_eq!(unforwarded(&with_try("Ok(None)")), 1);
    // 先试取、取不到就退回去等：带着转发的记号也不行，冻结时照样等满 5 秒。
    assert_eq!(
        unforwarded(&with_try(
            "match self.0.try_enter_operation()? { Some(permit) => Ok(Some(permit)), \
             None => self.0.enter_operation().map(Some) }"
        )),
        1
    );
    // 生产里的三种写法：看板包装层转发、真账本转不等待的固有方法、worker 客户端显式拒绝。
    assert_eq!(unforwarded(&with_try("self.0.try_enter_operation()")), 0);
    assert_eq!(unforwarded(&with_try("self.try_operation_permit()")), 0);
    assert_eq!(unforwarded(&with_try("Err(AuditError::Unavailable)")), 0);
}
