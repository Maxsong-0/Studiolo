//! 封存冻结窗口里不读写者回执（BUG-050）：回执随编辑数涨，逐条核只能在放开之后、导出的副本上做，
//! 核完才有写者的票据。169 上曾经在窗口里核，一次 11～26 秒，网站的写请求都在等。
//!
//! 按记号判定（注释、字符串不算）：
//! 1. 协调器的 `checkpoint.rs`（从冻结到放开）只对写者做「定格」「交出定格」两件事，写者句柄不许再传给别处；
//! 2. 窗口句柄 `FrozenWorkbench`、`FrozenBroker` 在产品代码里出现的地方、它们的 `impl` 块里用到的名字都是白名单：
//!    另开一个 `impl` 块、加一个收它的函数、块里调一个新名字都判红，换写法绕不过去；
//!    定格完的复核（两棵树逐个 stat、前后水位）一步不少——中间没有能插手的地方，用例碰不到，只能这样钉；
//! 3. 定格那个文件不碰回执、不读文件内容（读内容只在放开之后的拷贝里，一处）；
//! 4. 写者票据只有一处造、一处交出，交出之前把重放和回执都核过。
use crate::support::{is_test_fixture, rust_lexer::lex_product, rust_sources, workspace_root};
use std::{collections::BTreeMap, fs};

const COORDINATOR: &str = "crates/local-server/src/checkpoint.rs";
const WORKBENCH: &str = "crates/local-server/src/coordinator/checkpoint.rs";
const BROKER: &str = "crates/vault-broker/src/checkpoint.rs";
const CAPTURING: &str = "crates/vault-broker/src/checkpoint/capture.rs";
const EXPORTED: &str = "crates/vault-broker/src/checkpoint/exported.rs";

/// 窗口句柄在产品代码里出现的地方和次数：类型、字段、构造、返回、`impl`、转出。
const MENTIONS: [(&str, &str, usize); 7] = [
    ("FrozenBroker", WORKBENCH, 4),
    ("FrozenBroker", BROKER, 4),
    ("FrozenBroker", "crates/vault-broker/src/lib.rs", 1),
    ("FrozenWorkbench", COORDINATOR, 2),
    (
        "FrozenWorkbench",
        "crates/local-server/src/coordinator.rs",
        1,
    ),
    ("FrozenWorkbench", WORKBENCH, 5),
    ("FrozenWorkbench", "crates/local-server/src/lib.rs", 1),
];

/// 协调器里写者句柄的每一次出现（前一个记号｜后两个记号）：字段、冻结得来、装进结构、定格、交出定格、拆出来、放开、放进定格结果。
const COORDINATOR_USES: [&str; 8] = [
    ",|:FrozenWorkbench",
    "let|=self",
    ",|,_human_quiet",
    ".|.capture",
    ".|.take_capture",
    ",|,_human_quiet",
    "(|);",
    ",|:capture",
];

/// `impl FrozenBroker` 里用得到的名字。没有循环、没有回执、没有账本。
const BROKER_NAMES: &str = "Broker BrokerCapture BrokerError Budget CaptureChanged DATA_QUARANTINE Err FrozenBroker \
FsVaultBroker IncompleteCapture MutationUnresolved None Ok OwnerWatermark Result Some Sources VAULT_QUARANTINE \
binding broker budget capture capture_trees captured check check_deadline check_pair checkpoint_paths clone \
committed_through content_audit crate data deadline entries false fn from_process if impl is_empty let matches \
must_use mut new ok_or pair_id pub return root roots self source_data source_vault sources state store take \
take_capture thaw unresolved vault vault_uuid watermark write_status writer";
/// `impl FrozenWorkbench` 里用得到的名字。
const WORKBENCH_NAMES: &str = "BrokerCapture CheckpointError FrozenBroker FrozenWorkbench LocalError None Ok \
OwnerWatermark Result Some Unavailable _ as_mut as_ref broker broker_failure capture core error fn impl let map_err \
mut ok_or ok_or_else pub revalidate roots self source take_capture watermark workbench";
/// 还回写者的那一块（`impl Drop for FrozenWorkbench`）。
const DROP_NAMES: &str = "Drop FrozenBroker FrozenWorkbench PoisonError broker core drop fn for if impl into_inner \
is_none let lock map mut owner self std sync take thaw unwrap_or_else workbench";

/// 定格那个文件里不许出现的名字：回执、账本、持久记录的读取，和整段读内容的办法。
const CAPTURING_FORBIDDEN: [&str; 16] = [
    "records",
    "WriterRecords",
    "content_audit",
    "file_operation",
    "maintenance",
    "mutation",
    "history",
    "recorded",
    "replays",
    "audit",
    "disk",
    "scan",
    "Read",
    "BufReader",
    "read_to_end",
    "read_exact",
];

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

/// 去掉注释、字符串、空白之后的产品代码。
fn code(source: &str) -> String {
    lex_product(source).concat().replace(",)", ")")
}
fn names(tokens: &[String]) -> impl Iterator<Item = &String> {
    tokens
        .iter()
        .filter(|token| token.starts_with(|c: char| c == '_' || c.is_alphabetic()))
}

/// 记号序列里以这几个记号开头的 `impl` 块（到配对的右括号）。
fn implementation(source: &str, header: &[&str]) -> Option<Vec<String>> {
    let tokens = lex_product(source);
    let start = tokens
        .windows(header.len())
        .position(|window| window == header)?;
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

/// 全部产品源码（不含测试夹具）。
fn sources() -> BTreeMap<String, String> {
    let root = workspace_root();
    rust_sources(&root.join("crates"))
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(&root).unwrap().to_owned();
            let name = relative.to_str().unwrap().to_owned();
            (name.contains("/src/") && !is_test_fixture(&relative))
                .then(|| (name, fs::read_to_string(&path).unwrap()))
        })
        .collect()
}

/// 窗口句柄出现的地方和次数跟钉住的一样：多一个 `impl` 块、多一个收它的函数都会多一次。
fn mention_violation(sources: &BTreeMap<String, String>) -> Option<String> {
    let mut found = Vec::new();
    for name in ["FrozenBroker", "FrozenWorkbench"] {
        for (path, source) in sources.iter().filter(|(_, text)| text.contains(name)) {
            let count = lex_product(source)
                .iter()
                .filter(|token| *token == name)
                .count();
            if count > 0 {
                found.push((name, path.as_str(), count));
            }
        }
    }
    let mut expected = MENTIONS.to_vec();
    found.sort_unstable();
    expected.sort_unstable();
    (found != expected).then(|| {
        format!("窗口句柄出现的地方变了（窗口里不读回执；要加先想清楚再改 MENTIONS）：{found:?}")
    })
}

/// 协调器从冻结到放开的那个文件：写者句柄的每一次出现都是钉住的那几种，没有核对。
fn coordinator_violation(source: &str) -> Option<&'static str> {
    let tokens = lex_product(source);
    let uses = (1..tokens.len().saturating_sub(2))
        .filter(|&at| tokens[at] == "broker")
        .map(|at| format!("{}|{}{}", tokens[at - 1], tokens[at + 1], tokens[at + 2]))
        .collect::<Vec<_>>();
    if uses != COORDINATOR_USES {
        return Some("窗口里对写者只许 capture、take_capture 各一次，句柄不许再传给别处");
    }
    let reads = [
        "verify_exported",
        "receipt_matches",
        "ExportedBroker",
        "BrokerCheckpointAudit",
        "studiolo_vault_broker",
    ];
    tokens
        .iter()
        .any(|token| reads.contains(&token.as_str()))
        .then_some("窗口里出现了写者回执的核对")
}

/// 窗口里的写者句柄：有这样一段定格，用到的名字都在白名单里。
fn window_violation(
    source: &str,
    header: &[&str],
    signature: &str,
    allowed: &str,
) -> Option<String> {
    let Some(tokens) = implementation(source, header) else {
        return Some("找不到窗口里的写者句柄".to_owned());
    };
    if !tokens.concat().replace(",)", ")").contains(signature) {
        return Some("定格变了：它不许收核对回调，复核一步不能少".to_owned());
    }
    let allowed = allowed.split_whitespace().collect::<Vec<_>>();
    names(&tokens)
        .find(|name| !allowed.contains(&name.as_str()))
        .map(|name| {
            format!("窗口里的写者句柄多用了「{name}」：窗口里不读回执，要加先想清楚再改白名单")
        })
}

/// 定格那个文件：不碰回执，读内容只有放开之后的拷贝那一处。
fn capturing_violation(source: &str) -> Option<String> {
    let tokens = lex_product(source);
    if let Some(name) = names(&tokens).find(|name| CAPTURING_FORBIDDEN.contains(&name.as_str())) {
        return Some(format!(
            "定格里出现了「{name}」：窗口里不读回执、不读文件内容"
        ));
    }
    (tokens.iter().filter(|token| *token == "read_at").count() != 1)
        .then(|| "读内容只许在放开之后的拷贝里，一处".to_owned())
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

const BROKER_HEADER: [&str; 3] = ["impl", "FrozenBroker", "{"];
const WORKBENCH_HEADER: [&str; 3] = ["impl", "FrozenWorkbench", "{"];
const DROP_HEADER: [&str; 5] = ["impl", "Drop", "for", "FrozenWorkbench", "{"];

#[test]
fn the_frozen_window_never_reads_writer_receipts_and_only_verify_issues_the_ticket() {
    let sources = sources();
    assert_eq!(mention_violation(&sources), None);
    assert_eq!(coordinator_violation(&sources[COORDINATOR]), None);
    for (path, header, signature, allowed) in [
        (
            WORKBENCH,
            &WORKBENCH_HEADER[..],
            WORKBENCH_CAPTURE,
            WORKBENCH_NAMES,
        ),
        (
            WORKBENCH,
            &DROP_HEADER[..],
            "FrozenBroker::thaw",
            DROP_NAMES,
        ),
        (BROKER, &BROKER_HEADER[..], CAPTURE, BROKER_NAMES),
    ] {
        assert_eq!(
            window_violation(&sources[path], header, signature, allowed),
            None
        );
    }
    assert_eq!(capturing_violation(&sources[CAPTURING]), None);
    assert_eq!(ticket_violation(&sources[EXPORTED]), None);
    // 写者那个文件自己不造票据；票据只在 `ExportedBroker` 那里。
    assert!(!code(&sources[BROKER]).contains("SealedOwnerCheckpoint"));
}

/// 换写法绕：另开一个 `impl` 块、另写一个收句柄的函数、起别名，都多一次出现。
#[test]
fn the_guard_rejects_reaching_the_window_handles_from_another_place() {
    let sources = sources();
    for (path, extra) in [
        (
            "crates/vault-broker/src/checkpoint/audit.rs",
            "impl super::FrozenBroker { pub fn verify_now(&self) {} }",
        ),
        (
            WORKBENCH,
            "impl FrozenWorkbench { pub fn verify(&self) {} }",
        ),
        (BROKER, "pub fn check(frozen: &FrozenBroker) {}"),
        (
            "crates/vault-broker/src/checkpoint/replays.rs",
            "type Handle = super::FrozenBroker;",
        ),
        (
            "crates/vault-broker/src/maintenance.rs",
            "impl crate::FrozenBroker { pub fn verify_now(&self) {} }",
        ),
        (
            "crates/local-server/src/checkpoint/released.rs",
            "fn late(frozen: &crate::FrozenWorkbench) {}",
        ),
    ] {
        // 加在开头、加在末尾都一样：有的文件测试模块声明在中间，它后面的产品代码也要看。
        for added in [
            format!("{extra}\n{}", sources[path]),
            format!("{}\n{extra}", sources[path]),
        ] {
            let mut changed = sources.clone();
            changed.insert(path.to_owned(), added);
            assert!(mention_violation(&changed).is_some(), "{path}: {extra}");
        }
    }
    // 注释、测试模块里提到不算。
    let mut changed = sources.clone();
    let source = changed.get_mut(EXPORTED).unwrap();
    *source = format!(
        "// FrozenBroker 在窗口里\n{source}\n#[cfg(test)]\nmod more {{ fn f(b: FrozenBroker) {{}} }}\n"
    );
    assert_eq!(mention_violation(&changed), None);
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
        (
            "self.broker.capture()?;",
            "self.broker.capture()?; helper(&mut self.broker, &self.audit)?;",
        ),
        (
            "self.broker.capture()?;",
            "let handle = &mut self.broker; handle.capture()?;",
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
        coordinator_violation(&format!(
            "{source}\n// verify_exported(broker) 在放开之后\n"
        )),
        None
    );

    type Sample = (&'static str, &'static str);
    type Window = (
        &'static str,
        &'static [&'static str],
        &'static str,
        &'static str,
        &'static [Sample],
    );
    let windows: [Window; 2] = [
        (
            WORKBENCH,
            &WORKBENCH_HEADER,
            WORKBENCH_CAPTURE,
            WORKBENCH_NAMES,
            &[
                (
                    "pub fn capture(&mut self) ->",
                    "pub fn capture(&mut self, audit: &FrozenAudit) ->",
                ),
                (
                    "pub fn watermark(&self)",
                    "pub fn check(&self, audit: &FrozenAudit) {}\n    pub fn watermark(&self)",
                ),
                (
                    "pub fn watermark(&self)",
                    "pub fn check_receipts(&self) {}\n    pub fn watermark(&self)",
                ),
            ],
        ),
        (
            BROKER,
            &BROKER_HEADER,
            CAPTURE,
            BROKER_NAMES,
            &[
                (
                    "pub fn capture(&mut self) ->",
                    "pub fn capture(&mut self, verify: impl FnMut(u64)) ->",
                ),
                (
                    "self.captured = Some(capture);",
                    "replays::verify(&self.broker, self.deadline)?; self.captured = Some(capture);",
                ),
                // 名字不在旧黑名单里、块里别的函数里调的，也拦。
                (
                    "capture::check_pair(&vault, &data)?;",
                    "capture::check_pair(&vault, &data)?; \
                     crate::file_operation::checkpoint::verify(&self.broker, self.deadline, &mut |_, _, _| Ok(()))?;",
                ),
                (
                    "check_deadline(self.deadline)?;",
                    "for _ in 0..9 { self.broker.write_status()?; }",
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
    for (path, header, signature, allowed, samples) in windows {
        let source = read(path);
        for (before, after) in samples {
            assert!(source.contains(before), "{before}");
            let changed = source.replacen(before, after, 1);
            assert!(
                window_violation(&changed, header, signature, allowed).is_some(),
                "{after}"
            );
        }
        assert!(window_violation("", header, signature, allowed).is_some());
    }

    let source = read(CAPTURING);
    for (before, after) in [
        (
            "check_deadline(deadline)?;",
            "crate::maintenance::checkpoint::verify(broker, &mut record)?;",
        ),
        (
            "Node::File(held) => unchanged(held)?,",
            "Node::File(held) => { held.file.read_at(&mut [0], 0)?; }",
        ),
        (".read_at(&mut self.buffer, total)", ".metadata()"),
        (
            "use super::{",
            "use crate::records::WriterRecords;\nuse super::{",
        ),
    ] {
        assert!(source.contains(before), "{before}");
        let changed = source.replacen(before, after, 1);
        assert!(capturing_violation(&changed).is_some(), "{after}");
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
