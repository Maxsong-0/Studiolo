//! `scripts/test-linux.sh` 的可选开关 `STUDIOLO_TEST_RUNNER=nextest`（T-187）：只把「workspace 测试」
//! 那一行换成 nextest 加文档测试，其余门禁一条不少、顺序不变；不认识的值直接拒绝，不悄悄走默认。
//!
//! 用脚本自己的「只打印清单」模式（`STUDIOLO_TEST_PRINT_RUN=1`）核对，不连服务器。
//! 默认清单逐条等于 CI 的那部分由 `ci.rs` 管，这里只管开关换掉的是不是恰好那一行。

use crate::support::workspace_root;
use std::process::Command;

const SCRIPT: &str = "scripts/test-linux.sh";
const WORKSPACE_TEST: &str = "cargo --locked test --workspace";
const NEXTEST: [&str; 2] = [
    "cargo nextest run --locked --workspace",
    "cargo --locked test --workspace --doc",
];
/// 换成 nextest 时排在最前的那一行：服务器上没有 nextest 就在这里停下，不往后跑。
const NEXTEST_PRESENT: &str = "command -v cargo-nextest >/dev/null || ";

struct Printed {
    code: Option<i32>,
    lines: Vec<String>,
    stderr: String,
}

fn printed(runner: Option<&str>) -> Printed {
    let mut command = Command::new("bash");
    command
        .arg(workspace_root().join(SCRIPT))
        .env("STUDIOLO_TEST_PRINT_RUN", "1")
        .env_remove("STUDIOLO_TEST_RUNNER");
    if let Some(runner) = runner {
        command.env("STUDIOLO_TEST_RUNNER", runner);
    }
    let output = command.output().expect("bash 必须能跑 test-linux.sh");
    Printed {
        code: output.status.code(),
        lines: String::from_utf8(output.stdout)
            .expect("清单是 UTF-8")
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[test]
fn the_default_loop_is_unchanged_when_the_runner_is_unset_or_cargo() {
    let unset = printed(None);
    assert_eq!(unset.code, Some(0), "{}", unset.stderr);
    assert_eq!(
        unset
            .lines
            .iter()
            .filter(|line| *line == WORKSPACE_TEST)
            .count(),
        1,
        "默认回路里「workspace 测试」恰好一行"
    );
    assert!(
        unset.lines.iter().all(|line| !line.contains("nextest")),
        "默认回路不许出现 nextest：{:?}",
        unset.lines
    );
    assert_eq!(printed(Some("cargo")).lines, unset.lines);
}

#[test]
fn nextest_replaces_exactly_the_workspace_test_line() {
    let default = printed(None).lines;
    let swapped = printed(Some("nextest"));
    assert_eq!(swapped.code, Some(0), "{}", swapped.stderr);

    let at = default
        .iter()
        .position(|line| line == WORKSPACE_TEST)
        .expect("默认回路里有「workspace 测试」");
    // 被换掉的那一行之前、之后的门禁原样照旧。
    assert_eq!(swapped.lines[..at], default[..at]);
    assert_eq!(swapped.lines[at + 3..], default[at + 1..]);
    assert!(
        swapped.lines[at].starts_with(NEXTEST_PRESENT) && swapped.lines[at].ends_with("exit 1; }"),
        "先确认服务器上有 nextest，没有就停：{}",
        swapped.lines[at]
    );
    assert_eq!(swapped.lines[at + 1..at + 3], NEXTEST);
    assert!(
        swapped.lines.iter().all(|line| line != WORKSPACE_TEST),
        "换成 nextest 后不再整包跑 cargo test（文档测试另跑）"
    );
}

#[test]
fn an_unknown_runner_is_refused_instead_of_falling_back() {
    let refused = printed(Some("nextest "));
    assert_eq!(refused.code, Some(2));
    assert!(refused.lines.is_empty(), "拒绝时不打印清单");
    assert!(refused.stderr.contains("只认 cargo 或 nextest"));
}
