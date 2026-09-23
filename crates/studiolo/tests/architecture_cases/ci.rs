//! CI 入口逐行钉住门禁命令：整条逻辑命令行相等才算数，子串命中不算（旧写法的重言式：
//! `cargo --locked test` 会被路径安全那一行满足）。ci.yml 里的命令还必须落在指定作业、
//! 所在步骤与作业都不带 `if:`，也不 `continue-on-error`——否则门禁可以原样留着却不拦合并。
//! 命令一字不改也能让门禁失效的另有两类（T-138），同样判红：
//! - shell 语义：步骤、作业或全局 `defaults.run` 的 `shell` 不是恰好 `bash`（`sh`、`bash {0}`、
//!   `pwsh` 都不保证 errexit），或 run 块里一行 `set +e` 之类关掉 errexit——门禁失败后照样
//!   往下跑，步骤只看最后一条命令的退出码。test-linux.sh 同理：第一条门禁之前须有
//!   `set -euo pipefail`，全文不许关 errexit。
//! - 触发条件：`on:` 里 pull_request 缺失或被分支／路径／事件类型过滤，PR 上根本不跑门禁；
//!   push 不再恰只对 main，每次推 PR 分支都多跑一整套、挤占自托管 runner 那一条队列（ADR-020）。
//!
//! YAML 子集的解析在 `ci_yaml.rs`。

use crate::support::workspace_root;
use ci_yaml::{JOB_INDENT, disables_errexit, indent, logical_lines, trigger_gaps, workflow_steps};
use std::fs;

#[path = "ci_yaml.rs"]
mod ci_yaml;

const WORKFLOW: &str = ".github/workflows/ci.yml";
const LINUX: &str = "scripts/test-linux.sh";
const WORKSPACE_TEST: &str = "cargo --locked test --workspace";
/// test-linux.sh 须在第一条门禁之前开启的严格模式。
const STRICT: &str = "set -euo pipefail";
/// test-linux.sh 里有一行关掉 errexit 时报的缺口。
const ERREXIT_OFF: &str = "不许有 set +e、set +o errexit 一类关掉 errexit 的行";

/// ci.yml 的 rust 作业与 test-linux.sh 默认回路都要逐字执行的 Rust 门禁。
const SHARED: [&str; 4] = [
    "cargo --locked fmt --all -- --check",
    "cargo --locked clippy --workspace --all-targets -- -D warnings",
    WORKSPACE_TEST,
    "cargo --locked test -p studiolo-vault-broker --test linux_path_security",
];

/// 只在 ci.yml 的 rust 作业里钉住的生产特征构建。
const PRODUCTION: [&str; 2] = [
    "cargo --locked build -p studiolo-local-server --features studiolo-auth/production",
    "cargo --locked build -p studiolo-auth-admin --features production",
];

/// ci.yml 的 frontend 作业：不跑安装脚本的固定依赖安装，以及构建、测试、规范、格式四道门。
const FRONTEND: [&str; 5] = [
    "npm ci --ignore-scripts",
    "npm run build",
    "npm test",
    "npm run lint",
    "npm run format",
];

fn pinned() -> impl Iterator<Item = (&'static str, &'static str)> {
    let rust = SHARED
        .into_iter()
        .chain(PRODUCTION)
        .map(|command| ("rust", command));
    rust.chain(FRONTEND.into_iter().map(|command| ("frontend", command)))
}

/// ci.yml 的缺口：没有在指定作业中被无条件、整行执行的门禁（形如 `作业: 命令`），
/// 以及 `on:` 触发条件的缺口。
fn workflow_gaps(yaml: &str) -> Vec<String> {
    let steps = workflow_steps(yaml);
    pinned()
        .filter(|&(job, command)| {
            !steps.iter().any(|step| {
                step.job == job && !step.loose && step.run.iter().any(|line| line == command)
            })
        })
        .map(|(job, command)| format!("{job}: {command}"))
        .chain(trigger_gaps(yaml))
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
    gaps
}

fn entrypoints() -> (String, String) {
    let root = workspace_root();
    let read =
        |relative: &str| fs::read_to_string(root.join(relative)).expect("read CI entrypoint");
    (read(WORKFLOW), read(LINUX))
}

#[test]
fn ci_entrypoints_run_every_gate_as_an_unconditional_whole_command_line() {
    let (yaml, script) = entrypoints();
    assert_eq!(workflow_gaps(&yaml), Vec::<String>::new(), "{WORKFLOW}");
    assert_eq!(script_gaps(&script), Vec::<&str>::new(), "{LINUX}");
}

/// 红臂只改内存副本：定位目标行（整行相等；行内 `run:` 的命令也认）后交给 `edit` 改写。
fn mutate(text: &str, command: &str, edit: impl FnOnce(&mut Vec<String>, usize)) -> String {
    let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    let at = lines
        .iter()
        .position(|line| {
            let line = line.trim();
            line == command || line.strip_prefix("run:").map(str::trim) == Some(command)
        })
        .unwrap_or_else(|| panic!("找不到 `{command}`"));
    edit(&mut lines, at);
    lines.join("\n")
}

/// 向上找第一个满足条件的非注释行：命令所在步骤的 `- ` 行，或所在作业的作业名行。
fn enclosing(lines: &[String], at: usize, hit: impl Fn(&str) -> bool) -> usize {
    (0..at)
        .rev()
        .find(|&index| !lines[index].trim().starts_with('#') && hit(&lines[index]))
        .expect("命令属于某个步骤与作业")
}

fn step_start(lines: &[String], at: usize) -> usize {
    enclosing(lines, at, |line| line.trim().starts_with("- "))
}

/// 在 `owner` 行下插入更深一层的键；多行时后续行相对第一行缩进。
fn with_key(lines: &mut Vec<String>, owner: usize, key: &str) {
    let pad = " ".repeat(indent(&lines[owner]) + 2);
    let key = key.replace('\n', &format!("\n{pad}"));
    lines.insert(owner + 1, format!("{pad}{key}"));
}

#[test]
fn every_pinned_command_is_load_bearing_and_weakened_copies_turn_red() {
    let (yaml, script) = entrypoints();
    let red = [format!("rust: {WORKSPACE_TEST}")];
    // 删掉整个 Workspace 测试步骤：旧守卫的判据 `contains("cargo --locked test")` 仍被
    // 路径安全那一行满足（重言式），新守卫照样报缺。
    let dropped = mutate(&yaml, WORKSPACE_TEST, |lines, at| {
        let start = step_start(lines, at);
        let level = indent(&lines[start]);
        let end = (at + 1..lines.len())
            .find(|&index| !lines[index].trim().is_empty() && indent(&lines[index]) <= level)
            .unwrap_or(lines.len());
        lines.drain(start..end);
    });
    assert!(!dropped.contains(WORKSPACE_TEST) && dropped.contains("cargo --locked test"));
    assert_eq!(workflow_gaps(&dropped), red);
    // 每条钉住的命令都只由它自己那一行满足：删掉即报缺，且只报它。
    let remove = |lines: &mut Vec<String>, at: usize| {
        lines.remove(at);
    };
    for (job, command) in pinned() {
        let gaps = workflow_gaps(&mutate(&yaml, command, remove));
        assert_eq!(gaps, [format!("{job}: {command}")]);
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
        let rewrite = |lines: &mut Vec<String>, at: usize| {
            let pad = " ".repeat(indent(&lines[at]));
            lines[at] = weakened.replace('\n', &format!("\n{pad}"));
            lines[at].insert_str(0, &pad);
        };
        assert_eq!(workflow_gaps(&mutate(&yaml, WORKSPACE_TEST, rewrite)), red);
        let gaps = script_gaps(&mutate(&script, WORKSPACE_TEST, rewrite));
        assert_eq!(gaps, [WORKSPACE_TEST]);
    }
    // 步骤带条件、容错或换成不保证 errexit 的 shell：命令原样还在，但门禁可能不跑、跑挂了也不拦。
    for key in [
        "if: false",
        "continue-on-error: true",
        "'if': ${{ false }}",
        "shell: sh",
        "shell: bash {0}",
        "'shell': pwsh",
    ] {
        let gated = mutate(&yaml, WORKSPACE_TEST, |lines, at| {
            let start = step_start(lines, at);
            with_key(lines, start, key);
        });
        assert_eq!(workflow_gaps(&gated), red, "{key}");
    }
    // 作业级 `if:` 让整组前端门禁都不算数。
    let skipped = mutate(&yaml, "npm test", |lines, at| {
        let job = enclosing(lines, at, |line| indent(line) == JOB_INDENT);
        with_key(lines, job, "if: false");
    });
    let frontend = FRONTEND.map(|command| format!("frontend: {command}"));
    assert_eq!(workflow_gaps(&skipped), frontend);
}

#[test]
fn errexit_and_trigger_weakenings_turn_red() {
    let (yaml, script) = entrypoints();
    let red = [format!("rust: {WORKSPACE_TEST}")];
    let all = pinned()
        .map(|(job, command)| format!("{job}: {command}"))
        .collect::<Vec<_>>();
    let rust = &all[..SHARED.len() + PRODUCTION.len()];
    // run 块首行 `set +e`（行内 `run:` 先改写成字面块）：每条门禁都只让自己那一步报缺。
    for (job, command) in pinned() {
        let off = mutate(&yaml, command, |lines, at| {
            let pad = " ".repeat(indent(&lines[at]));
            if lines[at].trim().starts_with("run:") {
                lines[at] = format!("{pad}run: |\n{pad}  set +e\n{pad}  {command}");
            } else {
                let run = enclosing(lines, at, |line| line.trim().starts_with("run:"));
                lines.insert(run + 1, format!("{pad}set +e"));
            }
        });
        assert_eq!(workflow_gaps(&off), [format!("{job}: {command}")]);
    }
    // 步骤、rust 作业或全局 defaults 换了 shell：该步／该作业／全部门禁都不算数；
    // 显式写 `bash`（带不带引号）与省略时一样开着 errexit，照旧全绿。
    for (shell, step_red, job_red, top_red) in [
        ("bash {0}", &red[..], rust, &all[..]),
        ("sh", &red[..], rust, &all[..]),
        ("\"bash\"", &[][..], &[][..], &[][..]),
        ("bash", &[][..], &[][..], &[][..]),
    ] {
        let block = format!("defaults:\n  run:\n    shell: {shell}");
        let step = mutate(&yaml, WORKSPACE_TEST, |lines, at| {
            let start = step_start(lines, at);
            with_key(lines, start, &format!("shell: {shell}"));
        });
        let job = mutate(&yaml, "rust:", |lines, at| with_key(lines, at, &block));
        let top = mutate(&yaml, "jobs:", |lines, at| lines.insert(at, block.clone()));
        assert_eq!(workflow_gaps(&step), step_red, "步骤：{shell}");
        assert_eq!(workflow_gaps(&job), job_red, "作业 defaults：{shell}");
        assert_eq!(workflow_gaps(&top), top_red, "全局 defaults：{shell}");
    }
    // 把 `anchor` 那一行里的它换成 `new`（可含换行，空串即删掉这个键）后的 ci.yml 缺口。
    let gaps_after = |anchor: &str, new: &str| {
        let edit = |lines: &mut Vec<String>, at: usize| lines[at] = lines[at].replace(anchor, new);
        workflow_gaps(&mutate(&yaml, anchor, edit))
    };
    // 触发条件：PR 被过滤或不触发、push 放宽到别的分支或被路径过滤，门禁原样都在也不算数。
    for (anchor, new) in [
        ("pull_request:", "pull_request:\n    paths: [docs/**]"),
        ("pull_request:", ""),
        ("pull_request:", "pull_request:\n    types: [opened]"),
        ("branches: [main]", "branches: [\"**\"]"),
        ("push:", "push:\n    paths-ignore: [web/**]"),
        ("on:", "true:"),
    ] {
        let gaps = gaps_after(anchor, new);
        let triggers = gaps.iter().all(|gap| gap.starts_with("on"));
        assert!(!gaps.is_empty() && triggers, "{new}：{gaps:?}");
    }
    // 与现状等价的写法照旧全绿：带引号的 on、另两种写法的 [main]、含三种默认类型的 types。
    let typed = "pull_request:\n    types: [opened, synchronize, reopened, edited]";
    for (anchor, new) in [
        ("on:", "\"on\":"),
        ("branches: [main]", "branches: ['main']"),
        ("branches: [main]", "branches:\n      - \"main\""),
        ("pull_request:", typed),
    ] {
        assert_eq!(gaps_after(anchor, new), Vec::<String>::new(), "{new}");
    }
    // test-linux.sh：严格模式要在第一条门禁之前，全文不许关 errexit。
    let strict = |edit: fn(&mut Vec<String>, usize)| script_gaps(&mutate(&script, STRICT, edit));
    let off = strict(|lines, at| lines.insert(at + 1, "set +e".into()));
    assert_eq!(off, [ERREXIT_OFF]);
    assert_eq!(strict(|lines, at| lines[at].clear()), [STRICT]);
    assert_eq!(strict(|lines, at| lines[at..].rotate_left(1)), [STRICT]);
}

#[test]
fn errexit_switch_offs_follow_the_documented_pattern() {
    let off = "set +e|  set +eu|set +xe;|set  +o\terrexit|set +o errexit;";
    let kept = "set -e|set -euo pipefail|set +x|set +o pipefail|set +o errexits|set+e|echo set +e";
    for (lines, expected) in [(off, true), (kept, false)] {
        for line in lines.split('|') {
            assert_eq!(disables_errexit(line), expected, "{line}");
        }
    }
}
