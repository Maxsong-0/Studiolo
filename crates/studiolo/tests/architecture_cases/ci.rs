//! CI 入口逐行钉住门禁命令：整条逻辑命令行相等才算数，子串命中不算（旧写法的重言式：
//! `cargo --locked test` 会被路径安全那一行满足）。两个入口：
//! - `.testbox.toml` 的 ci 套（2026-11-16 起取代 GitHub CI，合进 main 之前必过）：每条门禁要落在 ci 套里
//!   指定目录的一步，这一步读得懂、不是后台服务、run 里没有关掉 errexit——否则门禁可以原样留着却不拦合并。
//!   测试箱按 `bash -eo pipefail -c` 跑每一步，errexit 默认开着。
//! - `scripts/test-linux.sh` 的默认回路：第一条门禁之前须有 `set -euo pipefail`，全文不许关 errexit，
//!   服务器上执行门禁的 `bash -euo pipefail -c` 那一行也逐字钉住（换成 `bash -c` 本机照旧严格、
//!   远端门禁却全部失效）。
//!
//! 步骤单的 TOML 子集解析在 `ci_steps.rs`。

use crate::support::workspace_root;
use ci_steps::{disables_errexit, logical_lines, steps};
use std::fs;

#[path = "ci_steps.rs"]
mod ci_steps;

const STEPS: &str = ".testbox.toml";
/// 合进 main 之前必过的那一套。
const SUITE: &str = "ci";
const LINUX: &str = "scripts/test-linux.sh";
const WORKSPACE_TEST: &str = "cargo --locked test --workspace";
/// test-linux.sh 须在第一条门禁之前开启的严格模式。
const STRICT: &str = "set -euo pipefail";
/// test-linux.sh 里有一处关掉 errexit 时报的缺口。
const ERREXIT_OFF: &str = "不许有 set +e、set +o errexit 一类关掉 errexit 的写法";
/// test-linux.sh 在服务器上真正执行门禁的那一行：远端 shell 也必须带 errexit。
const REMOTE: &str = r#"exec flock "$HOME/.cache/studiolo-locks/$STUDIOLO_NAME.lock" bash -euo pipefail -c "$STUDIOLO_RUN""#;

/// ci 套与 test-linux.sh 默认回路都要逐字执行的 Rust 门禁。
const SHARED: [&str; 6] = [
    "cargo --locked fmt --all -- --check",
    "cargo --locked clippy --workspace --all-targets -- -D warnings",
    WORKSPACE_TEST,
    "cargo --locked test -p studiolo-vault-broker --example t132-restore-probe",
    "cargo --locked test -p studiolo-vault-broker --test linux_path_security",
    "cargo --locked build -p studiolo-vault-admin",
];

/// 只在 ci 套里钉住的生产特征构建。
const PRODUCTION: [&str; 2] = [
    "cargo --locked build -p studiolo-local-server --features studiolo-auth/production",
    "cargo --locked build -p studiolo-auth-admin --features production",
];

/// 前端：不跑安装脚本的固定依赖安装，以及构建、测试、规范、格式四道门，都在 `web` 目录里跑。
const FRONTEND: [&str; 5] = [
    "npm ci --ignore-scripts",
    "npm run build",
    "npm test",
    "npm run lint",
    "npm run format",
];
const WEB: &str = "web";

/// 每条钉住的门禁和它必须所在的目录（`None` 是仓库根）。
fn pinned() -> impl Iterator<Item = (Option<&'static str>, &'static str)> {
    let rust = SHARED
        .into_iter()
        .chain(PRODUCTION)
        .map(|command| (None, command));
    rust.chain(FRONTEND.into_iter().map(|command| (Some(WEB), command)))
}

fn label(cwd: Option<&str>, command: &str) -> String {
    format!("{}: {command}", cwd.unwrap_or("."))
}

/// 门禁落在第几步（读得懂且没被削弱的那一步）；没有就是 `None`。
fn gate_step(toml: &str, cwd: Option<&str>, command: &str) -> Option<usize> {
    steps(toml).iter().position(|step| {
        step.suite == SUITE
            && !step.loose
            && step.cwd.as_deref() == cwd
            && step.run.iter().any(|line| line == command)
    })
}

/// 步骤单的缺口：没有在 ci 套里指定目录、读得懂且没被削弱的一步中整行执行的门禁（形如 `目录: 命令`）。
fn suite_gaps(toml: &str) -> Vec<String> {
    pinned()
        .filter(|&(cwd, command)| gate_step(toml, cwd, command).is_none())
        .map(|(cwd, command)| label(cwd, command))
        .collect()
}

/// test-linux.sh 默认回路的缺口：缺少的共享 Rust 门禁（按整条逻辑命令行比对）；第一条门禁
/// 之前没有 `set -euo pipefail`；或有哪一行关掉了 errexit。
fn script_gaps(script: &str) -> Vec<&'static str> {
    let lines = logical_lines(script.lines().map(str::trim));
    let mut gaps = SHARED
        .into_iter()
        .filter(|command| !lines.iter().any(|line| line == command))
        .collect::<Vec<_>>();
    let first = lines
        .iter()
        .position(|line| SHARED.contains(&line.as_str()))
        .unwrap_or(lines.len());
    if !lines[..first].iter().any(|line| line == STRICT) {
        gaps.push(STRICT);
    }
    if script.lines().any(disables_errexit) {
        gaps.push(ERREXIT_OFF);
    }
    if !lines.iter().any(|line| line == REMOTE) {
        gaps.push(REMOTE);
    }
    gaps
}

fn entrypoints() -> (String, String) {
    let root = workspace_root();
    let read =
        |relative: &str| fs::read_to_string(root.join(relative)).expect("read CI entrypoint");
    (read(STEPS), read(LINUX))
}

#[test]
fn ci_entrypoints_run_every_gate_as_an_unconditional_whole_command_line() {
    let (toml, script) = entrypoints();
    assert_eq!(suite_gaps(&toml), Vec::<String>::new(), "{STEPS}");
    assert_eq!(script_gaps(&script), Vec::<&str>::new(), "{LINUX}");
}

/// 红臂只改内存副本：定位目标行（去首尾空白后整行相等）后交给 `edit` 改写。
fn mutate(text: &str, target: &str, edit: impl FnOnce(&mut Vec<String>, usize)) -> String {
    let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    let at = lines
        .iter()
        .position(|line| line.trim() == target)
        .unwrap_or_else(|| panic!("找不到 `{target}`"));
    edit(&mut lines, at);
    lines.join("\n")
}

/// 向上找第一行满足条件的：命令所在步骤的表头，或所在 run 块的开头。
fn above(lines: &[String], at: usize, hit: impl Fn(&str) -> bool) -> usize {
    (0..at)
        .rev()
        .find(|&index| hit(lines[index].trim()))
        .expect("命令属于某一步")
}

fn header(lines: &[String], at: usize) -> usize {
    above(lines, at, |line| line.starts_with("[["))
}

/// 命令所在这一步之后的第一个步骤表头（没有就是文件末尾）。
fn next_header(lines: &[String], at: usize) -> usize {
    (at + 1..lines.len())
        .find(|&index| lines[index].trim().starts_with("[["))
        .unwrap_or(lines.len())
}

/// 同一步里的门禁（它们共进退）。
fn same_step(toml: &str, command: &str) -> Vec<String> {
    let (cwd, _) = pinned()
        .find(|&(_, pinned)| pinned == command)
        .expect("钉住的门禁");
    let step = gate_step(toml, cwd, command);
    pinned()
        .filter(|&(cwd, other)| gate_step(toml, cwd, other) == step)
        .map(|(cwd, other)| label(cwd, other))
        .collect()
}

#[test]
fn every_pinned_command_is_load_bearing_and_weakened_copies_turn_red() {
    let (toml, script) = entrypoints();
    let red = [label(None, WORKSPACE_TEST)];
    // 删掉整个 Workspace 测试步骤：旧守卫的判据 `contains("cargo --locked test")` 仍被
    // 路径安全那一行满足（重言式），新守卫照样报缺。
    let dropped = mutate(&toml, WORKSPACE_TEST, |lines, at| {
        let (start, end) = (header(lines, at), next_header(lines, at));
        lines.drain(start..end);
    });
    assert!(!dropped.contains(WORKSPACE_TEST) && dropped.contains("cargo --locked test"));
    assert_eq!(suite_gaps(&dropped), red);
    // 每条钉住的命令都只由它自己那一行满足：删掉即报缺，且只报它。
    let remove = |lines: &mut Vec<String>, at: usize| {
        lines.remove(at);
    };
    for (cwd, command) in pinned() {
        assert_eq!(
            suite_gaps(&mutate(&toml, command, remove)),
            [label(cwd, command)]
        );
    }
    for command in SHARED {
        assert_eq!(script_gaps(&mutate(&script, command, remove)), [command]);
    }
    // 注释掉、改参数、吞失败、被续行拼进别的命令，都不再算执行了这条门禁。
    for weakened in [
        format!("# {WORKSPACE_TEST}"),
        format!("{WORKSPACE_TEST} --no-run"),
        format!("{WORKSPACE_TEST} || true"),
        format!("echo \\\n{WORKSPACE_TEST}"),
    ] {
        let rewrite = |lines: &mut Vec<String>, at: usize| lines[at] = weakened.clone();
        assert_eq!(suite_gaps(&mutate(&toml, WORKSPACE_TEST, rewrite)), red);
        let gaps = script_gaps(&mutate(&script, WORKSPACE_TEST, rewrite));
        assert_eq!(gaps, [WORKSPACE_TEST]);
    }
    // 这一步换了套、改成后台服务、换了目录、加了守卫读不懂的键：命令原样还在，但门禁可能不跑、跑挂了也不拦。
    for key in [
        "service = true",
        "cwd = \"crates\"",
        "env = { CARGO_TARGET_DIR = \"/tmp\" }",
        "run_as = \"nobody\"",
    ] {
        let changed = mutate(&toml, WORKSPACE_TEST, |lines, at| {
            let start = header(lines, at);
            lines.insert(start + 1, key.to_owned());
        });
        assert_eq!(suite_gaps(&changed), red, "{key}");
    }
    // 紧跟在这一步后面的子表在 TOML 里把键挂到这一步上（这里改 PATH，cargo 可以被换掉）。
    let nested = mutate(&toml, WORKSPACE_TEST, |lines, at| {
        let end = next_header(lines, at);
        lines.insert(end, "[suites.ci.steps.env]".to_owned());
        lines.insert(end + 1, "PATH = \"/tmp\"".to_owned());
    });
    assert_eq!(suite_gaps(&nested), red);
    let moved = mutate(&toml, WORKSPACE_TEST, |lines, at| {
        let start = header(lines, at);
        lines[start] = "[[suites.nightly.steps]]".to_owned();
    });
    assert_eq!(suite_gaps(&moved), red);
    // 前端那几步离开 web 目录也不算数。
    let elsewhere = mutate(&toml, "npm test", |lines, at| {
        let start = header(lines, at);
        let cwd = (start..at)
            .find(|&index| lines[index].trim() == "cwd = \"web\"")
            .expect("前端步骤写明了目录");
        lines.remove(cwd);
    });
    assert_eq!(suite_gaps(&elsewhere), [label(Some(WEB), "npm test")]);
}

#[test]
fn errexit_weakenings_turn_red() {
    let (toml, script) = entrypoints();
    // run 块开头 `set +e`：这一步的门禁（只有它们）都报缺。
    for (_, command) in pinned() {
        let off = mutate(&toml, command, |lines, at| {
            let run = above(lines, at, |line| line == "run = '''");
            lines.insert(run + 1, "set +e".to_owned());
        });
        assert_eq!(suite_gaps(&off), same_step(&toml, command), "{command}");
    }
    // test-linux.sh：严格模式要在第一条门禁之前，全文不许关 errexit。
    let strict = |edit: fn(&mut Vec<String>, usize)| script_gaps(&mutate(&script, STRICT, edit));
    let off = strict(|lines, at| lines.insert(at + 1, "set +e".into()));
    assert_eq!(off, [ERREXIT_OFF]);
    assert_eq!(strict(|lines, at| lines[at].clear()), [STRICT]);
    assert_eq!(strict(|lines, at| lines[at..].rotate_left(1)), [STRICT]);
    // 远端那行换成不带 errexit 的 bash -c，本机脚本照旧严格，服务器上的门禁却全部失效。
    let remote = script.replace("bash -euo pipefail -c", "bash -c");
    assert_eq!(script_gaps(&remote), [REMOTE]);
}
