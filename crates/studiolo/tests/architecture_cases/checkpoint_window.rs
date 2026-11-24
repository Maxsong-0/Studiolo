//! 封存冻结窗口里不读写者回执（BUG-050）：回执随编辑数涨，逐条核只能在放开之后、导出的副本上做，
//! 核完才有写者的票据。169 上曾经在窗口里核，一次 11～26 秒，网站的写请求都在等。
//!
//! 按记号判定（注释、字符串不算），钉住三处窗口代码和一处出票：
//! 1. 协调器的 `checkpoint.rs`（从冻结到放开）只对写者做「定格」「交出定格」两件事；
//! 2. `FrozenWorkbench`、`FrozenBroker` 的接口不收核对回调、不碰账本和回执；
//!    定格完的复核（两棵树逐个 stat、前后水位）一步不少——中间没有能插手的地方，用例碰不到，只能这样钉；
//! 3. 写者票据只有一处造、一处交出，交出之前把重放和回执都核过。
use crate::support::{rust_lexer::lex, workspace_root};
use std::fs;

const COORDINATOR: &str = "crates/local-server/src/checkpoint.rs";
const WORKBENCH: &str = "crates/local-server/src/coordinator/checkpoint.rs";
const BROKER: &str = "crates/vault-broker/src/checkpoint.rs";
const EXPORTED: &str = "crates/vault-broker/src/checkpoint/exported.rs";

/// 写者的定格：水位、定格、租约、两棵树逐个复核、再查一次水位，全对上才留下。
const CAPTURE: &str = "pubfncapture(&mutself)->Result<(),BrokerError>{self.captured=None;\
letwatermark=self.watermark()?;letcapture=self.capture_trees(watermark)?;\
self.broker.writer.check(&self.broker.root,&self.broker.data)?;\
capture.vault.matches(&self.broker.root,VAULT_QUARANTINE,self.deadline)?;\
capture.data.matches(&self.broker.data,DATA_QUARANTINE,self.deadline)?;\
ifcapture.watermark!=self.watermark()?{returnErr(BrokerError::CaptureChanged);}\
self.captured=Some(capture);Ok(())}";
/// 工作台那一层的定格：核一遍根，再转给写者。
const WORKBENCH_CAPTURE: &str = "pubfncapture(&mutself)->Result<(),CheckpointError>{";

/// 窗口代码里不许出现的记号：核对回调、账本、回执。
const FORBIDDEN: [&str; 12] = [
    "Fn",
    "FnMut",
    "FnOnce",
    "FrozenAudit",
    "ExportedBroker",
    "BrokerCheckpointAudit",
    "BrokerCheckpointCoverage",
    "verify_exported",
    "receipt_matches",
    "audit",
    "replays",
    "SealedOwnerCheckpoint",
];

/// 去掉注释、字符串、空白之后的产品代码。
fn code(source: &str) -> String {
    let product = source
        .find("#[cfg(test)]\nmod ")
        .map_or(source, |at| &source[..at]);
    lex(product).concat().replace(",)", ")")
}

/// 记号序列里某个 `impl 名字 {` 的整段（到配对的右括号）。
fn implementation(source: &str, name: &str) -> Option<Vec<String>> {
    let tokens = lex(source);
    let start = tokens
        .windows(3)
        .position(|window| window == ["impl", name, "{"])?;
    let mut depth = 0_usize;
    for (offset, token) in tokens[start..].iter().enumerate() {
        match token.as_str() {
            "{" => depth += 1,
            "}" if depth == 1 => return Some(tokens[start..=start + offset].to_vec()),
            "}" => depth -= 1,
            _ => {}
        }
    }
    None
}

/// 协调器从冻结到放开的那个文件：对写者只定格、交出定格。
fn coordinator_violation(source: &str) -> Option<&'static str> {
    let code = code(source);
    if code.matches("self.broker.").count() != 2
        || !code.contains("self.broker.capture()?;")
        || !code.contains("self.broker.take_capture()")
    {
        return Some("窗口里对写者只许 capture、take_capture 各一次");
    }
    let reads = [
        "verify_exported",
        "receipt_matches",
        "ExportedBroker",
        "BrokerCheckpointAudit",
        "studiolo_vault_broker",
    ];
    lex(source)
        .iter()
        .any(|token| reads.contains(&token.as_str()))
        .then_some("窗口里出现了写者回执的核对")
}

/// 窗口里的写者句柄：有这样一段定格，没有回调、账本和回执。
fn window_violation(source: &str, name: &str, signature: &str) -> Option<&'static str> {
    let Some(tokens) = implementation(source, name) else {
        return Some("找不到窗口里的写者句柄");
    };
    if !tokens.concat().replace(",)", ")").contains(signature) {
        return Some("定格变了：它不许收核对回调，复核一步不能少");
    }
    tokens
        .iter()
        .any(|token| FORBIDDEN.contains(&token.as_str()))
        .then_some("窗口里的写者句柄碰了回执、账本或收了回调")
}

/// 票据：只在 `ticket` 里造、只由 `verify` 交出，交出之前重放和回执都核过。
fn ticket_violation(source: &str) -> Option<&'static str> {
    let code = code(source);
    if !code.contains(
        "self.layout()?;replays::verify(&self,deadline)?;letcoverage=audit::verify(&self,deadline,&mutverify)?;self.layout()?;check_deadline(deadline)?;Ok((self.ticket(),coverage))",
    ) {
        return Some("verify 出票之前没有照顺序核完布局、重放、回执");
    }
    // `ticket` 的返回类型加函数体里的那一处，和 `verify` 的返回类型、`use` 一处，都不是另一个出口。
    if code.matches("self.ticket()").count() != 1
        || code.matches("SealedOwnerCheckpoint{").count() != 2
        || code.matches("SealedOwnerCheckpoint").count() != 4
        || code.contains("pubfnticket")
        || code.contains(")fnticket")
    {
        return Some("写者票据多了一个出口");
    }
    None
}

#[test]
fn the_frozen_window_never_reads_writer_receipts_and_only_verify_issues_the_ticket() {
    let read = |path: &str| fs::read_to_string(workspace_root().join(path)).unwrap();
    assert_eq!(coordinator_violation(&read(COORDINATOR)), None);
    assert_eq!(
        window_violation(&read(WORKBENCH), "FrozenWorkbench", WORKBENCH_CAPTURE),
        None
    );
    assert_eq!(
        window_violation(&read(BROKER), "FrozenBroker", CAPTURE),
        None
    );
    assert_eq!(ticket_violation(&read(EXPORTED)), None);
    // 写者那个文件自己不造票据；票据只在 `ExportedBroker` 那里。
    assert!(!code(&read(BROKER)).contains("SealedOwnerCheckpoint"));
}

#[test]
fn the_guard_rejects_moving_the_receipts_back_into_the_window() {
    let read = |path: &str| fs::read_to_string(workspace_root().join(path)).unwrap();
    let source = read(COORDINATOR);
    for (before, after) in [
        (
            "self.broker.capture()?;",
            "self.broker.capture()?; verify_exported(exported, &self.audit, deadline)?;",
        ),
        ("self.broker.capture()?;", ""),
        (
            "self.broker.capture()?;",
            "self.broker.verify_audit(&self.audit)?;",
        ),
        (
            "self.broker.capture()?;",
            "self.broker.capture()?; self.broker.verify_audit(&self.audit)?;",
        ),
        ("fn add(", "fn receipt_matches() {}\nfn add("),
        (
            "use std::{",
            "use studiolo_vault_broker::BrokerCapture;\nuse std::{",
        ),
    ] {
        assert!(source.contains(before), "{before}");
        let changed = source.replacen(before, after, 1);
        assert!(coordinator_violation(&changed).is_some(), "{after}");
    }
    // 注释里提到不算。
    assert_eq!(
        coordinator_violation(&format!("{source}\n// verify_exported 在放开之后\n")),
        None
    );

    type Sample = (&'static str, &'static str);
    let windows: [(&str, &str, &str, &[Sample]); 2] = [
        (
            WORKBENCH,
            "FrozenWorkbench",
            WORKBENCH_CAPTURE,
            &[
                (
                    "pub fn capture(&mut self) ->",
                    "pub fn capture(&mut self, audit: &FrozenAudit) ->",
                ),
                (
                    "pub fn watermark(&self)",
                    "pub fn check(&self, audit: &FrozenAudit) {}\n    pub fn watermark(&self)",
                ),
            ],
        ),
        (
            BROKER,
            "FrozenBroker",
            CAPTURE,
            &[
                (
                    "pub fn capture(&mut self) ->",
                    "pub fn capture(&mut self, verify: impl FnMut(u64)) ->",
                ),
                (
                    "self.captured = Some(capture);",
                    "replays::verify(&self.broker, self.deadline)?; self.captured = Some(capture);",
                ),
                (
                    "if capture.watermark != self.watermark()? {",
                    "if capture.watermark != capture.watermark {",
                ),
                (
                    ".matches(&self.broker.data, DATA_QUARANTINE, self.deadline)?;",
                    ".is_file();",
                ),
                (
                    ".matches(&self.broker.root, VAULT_QUARANTINE, self.deadline)?;",
                    ".is_file();",
                ),
            ],
        ),
    ];
    for (path, name, signature, samples) in windows {
        let source = read(path);
        for (before, after) in samples {
            assert!(source.contains(before), "{before}");
            let changed = source.replacen(before, after, 1);
            assert!(
                window_violation(&changed, name, signature).is_some(),
                "{after}"
            );
        }
        assert!(window_violation("", name, signature).is_some());
    }

    let source = read(EXPORTED);
    for (before, after) in [
        ("replays::verify(&self, deadline)?;", ""),
        (
            "audit::verify(&self, deadline, &mut verify)?;",
            "BrokerCheckpointCoverage::default();",
        ),
        (
            "let coverage",
            "if deadline == Instant::now() { return Ok((self.ticket(), todo!())); }\n        let coverage",
        ),
        (
            "fn ticket(&self)",
            "pub fn quick(&self) -> SealedOwnerCheckpoint { self.watermark.clone().into() }\n    fn ticket(&self)",
        ),
        ("    fn ticket(&self)", "    pub fn ticket(&self)"),
        ("    fn ticket(&self)", "    pub(crate) fn ticket(&self)"),
        ("check_deadline(deadline)?;", ""),
    ] {
        assert!(source.contains(before), "{before}");
        let changed = source.replacen(before, after, 1);
        assert!(ticket_violation(&changed).is_some(), "{after}");
    }
}
