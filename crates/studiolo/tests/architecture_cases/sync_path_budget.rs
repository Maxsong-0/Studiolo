//! 同步路径预算只有一处定义（BUG-047）：原来五个 crate 各写一份 4096，只放宽一处会把失败挪到下一处。
//! 这里钉住四件事：常量只在 `studiolo-core` 定义一次；链上各环节引用它、不再出现裸的预算字面量；
//! 人侧在第二次 canonical 观察之前先拿三方并集和预算比；人侧启动打开 Bridge 后、组装平面前先把持久预算升上去。
use crate::support::rust_lexer::lex;
use crate::support::{rust_sources, workspace_root};
use std::{fs, path::Path};

/// 唯一定义所在的文件。
const DEFINITION: &str = "crates/core/src/sync_budget.rs";

/// 同步链上按路径预算截止的每一处；都必须引用常量（或由它派生的 `MAX_PATHS`）。
const BUDGETED: [&str; 12] = [
    "crates/vault-broker/src/sync_observation/selection.rs",
    "crates/vault-broker/src/sync_observation/kernel.rs",
    "crates/vault-broker/src/sync_observation/metadata.rs",
    "crates/host-config/src/sync_scope/union.rs",
    "crates/host-config/src/sync_scope/paths.rs",
    "crates/sync-bridge/src/validation.rs",
    "crates/sync-bridge/src/evidence.rs",
    "crates/sync-bridge/src/sqlite/mod.rs",
    "crates/sync-bridge/src/sqlite/dbload.rs",
    "crates/sync-bridge/src/sqlite/validate.rs",
    "crates/local-server/src/sync_plane/mirror/observe.rs",
    "crates/local-server/src/startup/health_input.rs",
];

/// 人侧观察两侧的那个文件；第二次 canonical 观察前必须先比预算。
const WITNESS: &str = "crates/local-server/src/sync_plane/witness.rs";

/// 人侧启动装配同步平面的那个文件；没有真传输后端装不起来，接线只能在这里钉。
const STARTUP: &str = "crates/local-server/src/startup/sync_input.rs";

/// 去掉文件末尾的内联测试模块；测试里写具体条数是正常的，只看产品代码。
fn production(source: &str) -> &str {
    source
        .find("#[cfg(test)]\nmod ")
        .map_or(source, |at| &source[..at])
}

/// 整数字面量可带的类型后缀；剥掉后再比数值。
const INTEGER_SUFFIXES: [&str; 8] = ["usize", "isize", "u128", "u64", "u32", "i128", "i64", "i32"];

/// 产品代码里裸写的预算字面量（旧值 4096、新值 16384，带不带下划线、类型后缀都算）。
fn bare_budget_literals(source: &str) -> Vec<String> {
    lex(production(source))
        .into_iter()
        .filter(|token| {
            let body = INTEGER_SUFFIXES
                .iter()
                .find_map(|suffix| token.strip_suffix(suffix))
                .unwrap_or(token);
            let digits = body.replace('_', "");
            token.starts_with(|c: char| c.is_ascii_digit())
                && (digits == "4096" || digits == "16384")
        })
        .collect()
}

/// 产品代码有没有引用预算常量（或 Bridge 库里由它派生的 `MAX_PATHS`）。
fn references_budget(source: &str) -> bool {
    lex(production(source))
        .iter()
        .any(|token| token == "SYNC_PATH_BUDGET" || token == "MAX_PATHS")
}

/// 第二次 canonical 观察之前有没有先比预算；返回违规说明。
fn witness_violation(source: &str) -> Option<&'static str> {
    let compact = production(source).split_whitespace().collect::<String>();
    let check = compact.find("budget::check(&self.plane.health,union.len())?;");
    let second = compact.find("self.observe_canonical(&union)");
    match (check, second) {
        (None, _) => Some("observe_pair 里没有 budget::check"),
        (_, None) => Some("observe_pair 里找不到按并集的第二次 canonical 观察"),
        (Some(check), Some(second)) if check > second => {
            Some("budget::check 必须在第二次 canonical 观察之前")
        }
        _ => None,
    }
}

/// 打开 Bridge 之后、组装平面之前有没有按本发行的保守预算升持久预算；返回违规说明。
fn startup_violation(source: &str) -> Option<&'static str> {
    let compact = production(source).split_whitespace().collect::<String>();
    let open = compact.find("BridgeState::open_existing(");
    let target = compact.find("letlimits=bridge_limits(scope);");
    let raise = compact.find(".raise_capacity(limits)");
    let plane = compact.find("SyncPlane::open(");
    match (open, target, raise, plane) {
        (_, None, _, _) => Some("升级目标必须是 bridge_limits(scope)"),
        (_, _, None, _) => Some("打开 Bridge 后没有 raise_capacity"),
        (Some(open), Some(_), Some(raise), Some(plane)) if open < raise && raise < plane => None,
        _ => Some("raise_capacity 必须在打开 Bridge 之后、组装平面之前"),
    }
}

#[test]
fn startup_raises_the_persisted_bridge_budget_before_assembling_the_plane() {
    let root = workspace_root();
    let source = fs::read_to_string(root.join(STARTUP)).unwrap();
    assert_eq!(startup_violation(&source), None);

    let raise = ".raise_capacity(limits)";
    assert!(source.contains(raise), "{STARTUP} 的写法变了，守卫要跟着改");
    assert_eq!(
        startup_violation(&source.replace(raise, ".view()")),
        Some("打开 Bridge 后没有 raise_capacity"),
        "去掉升级要拦下"
    );
    assert_eq!(
        startup_violation(&source.replace(
            "let limits = bridge_limits(scope);",
            "let limits = studiolo_sync_bridge::Limits::conservative(0);"
        )),
        Some("升级目标必须是 bridge_limits(scope)"),
        "目标换成别的来源要拦下"
    );
}

#[test]
fn sync_path_budget_is_defined_once_in_core() {
    let root = workspace_root();
    let mut definitions = Vec::new();
    for path in rust_sources(&root.join("crates")) {
        let tokens = lex(&fs::read_to_string(&path).unwrap());
        if tokens
            .windows(2)
            .any(|pair| pair[0] == "const" && pair[1] == "SYNC_PATH_BUDGET")
        {
            definitions.push(path.strip_prefix(&root).unwrap().to_path_buf());
        }
    }
    assert_eq!(
        definitions,
        vec![Path::new(DEFINITION).to_path_buf()],
        "同步路径预算只许在 studiolo-core 定义一次，其余地方引用 studiolo_core::SYNC_PATH_BUDGET"
    );
}

#[test]
fn every_budgeted_step_references_the_constant_and_has_no_bare_limit() {
    let root = workspace_root();
    for path in BUDGETED {
        let source = fs::read_to_string(root.join(path)).unwrap();
        assert!(
            references_budget(&source),
            "{path} 按路径预算截止，必须引用 studiolo_core::SYNC_PATH_BUDGET（BUG-047）"
        );
        assert_eq!(
            bare_budget_literals(&source),
            Vec::<String>::new(),
            "{path} 不许裸写路径预算；改引用 studiolo_core::SYNC_PATH_BUDGET（BUG-047）"
        );
    }
}

#[test]
fn bare_limit_detection_catches_literals_but_not_comments_strings_or_tests() {
    for planted in [
        "const MAX: usize = 4096;",
        "if n > 4_096 { }",
        "let limit = 4096usize;",
        "let limit = 16_384_u64;",
        "const FILE_LIMIT: usize = 16384;",
    ] {
        assert!(
            !bare_budget_literals(planted).is_empty(),
            "应当拦下：{planted}"
        );
    }
    for allowed in [
        "// 预算 4096 时是 1 MiB",
        "let text = \"4096\";",
        "const NAMES: usize = 65_536;",
        "const PAGE: usize = 40960;",
        "fn f() {}\n#[cfg(test)]\nmod tests { const N: usize = 4096; }",
    ] {
        assert!(
            bare_budget_literals(allowed).is_empty(),
            "不该拦：{allowed}"
        );
    }
    assert!(references_budget(
        "let n = studiolo_core::SYNC_PATH_BUDGET;"
    ));
    assert!(references_budget("if out.len() >= super::MAX_PATHS {}"));
    assert!(!references_budget(
        "// SYNC_PATH_BUDGET\nconst MAX: usize = 4096;"
    ));
}

#[test]
fn witness_checks_the_union_against_the_budget_before_the_second_canonical_observation() {
    let root = workspace_root();
    let source = fs::read_to_string(root.join(WITNESS)).unwrap();
    assert_eq!(witness_violation(&source), None);

    let check = "budget::check(&self.plane.health, union.len())?;";
    assert!(source.contains(check), "{WITNESS} 的写法变了，守卫要跟着改");
    let removed = source.replace(check, "");
    assert!(witness_violation(&removed).is_some(), "去掉比对要拦下");
    let moved = removed.replace(
        "canonical = at(Step::Canonical, self.observe_canonical(&union))?;",
        &format!("canonical = at(Step::Canonical, self.observe_canonical(&union))?;\n{check}"),
    );
    assert_ne!(moved, removed);
    assert_eq!(
        witness_violation(&moved),
        Some("budget::check 必须在第二次 canonical 观察之前"),
        "挪到第二次观察之后要拦下"
    );
}
