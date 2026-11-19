//! CI 入口逐行钉住门禁命令：整条逻辑命令行相等才算数，子串命中不算（旧写法的重言式：
//! `cargo --locked test` 会被路径安全那一行满足）。两个入口：
//! - `.testbox.toml` 的 ci 套（2026-11-16 起取代 GitHub CI，合进 main 之前必过）：每条门禁要落在 ci 套里
//!   指定目录的一步，这一步读得懂、不是后台服务、run 里没有关掉 errexit——否则门禁可以原样留着却不拦合并。
//!   测试箱按 `bash -eo pipefail -c` 跑每一步，errexit 默认开着。这一步的第一行还得是对应目录的环境脚本
//!   （Worker 那几步带 Node 版本），最后一条门禁之前只许有别的门禁和几行版本诊断：`exit`、`if`、`cd`、
//!   改 PATH、定义同名函数这些插在门禁前面，门禁就可能不跑或换了东西跑（复审 A4-F2、A4-F3）。最后一条门禁
//!   之后的行改变不了门禁的结果（门禁失败当场就退出），不管。
//! - `scripts/test-linux.sh` 的默认回路：第一条门禁之前须有 `set -euo pipefail`，全文不许关 errexit，
//!   服务器上执行门禁的 `bash -euo pipefail -c` 那一行也逐字钉住（换成 `bash -c` 本机照旧严格、
//!   远端门禁却全部失效）。
//!
//! 步骤单的 TOML 子集解析在 `ci_steps.rs`。

use crate::support::workspace_root;
use ci_steps::{Step, disables_errexit, logical_lines, steps};
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

/// 只在 ci 套里钉住、要 release 构建才量得准的容量契约（T-215，vault BUG-039）：Markdown 合计上限满额时
/// 16 路并发读的时延与常驻内存。debug 下它标为忽略，Workspace 测试那一步跑不到它；两个运行包也都不带它
/// （`package-agent.sh`、`package-preview.sh` 的 `PACKAGE_EXCLUDED_TESTS`，生产主机上不跑满额负载）。删掉这一行，上限调高了
/// 扛不住、或者整读校验又变成随篇数超线性增长，就没有任何一步会拦。
const RELEASE: [&str; 1] =
    ["cargo --locked test --release -p studiolo-agent-server --test replica_capacity_contract"];

/// 前端：不跑安装脚本的固定依赖安装，以及构建、测试、规范、格式四道门，都在 `web` 目录里跑。
const FRONTEND: [&str; 5] = [
    "npm ci --ignore-scripts",
    "npm run build",
    "npm test",
    "npm run lint",
    "npm run format",
];
const WEB: &str = "web";

/// 仓库根的其余回归（部署脚本、告警监控、副本管线、本地登记工具 CLI），逐行钉住：删掉一条
/// 就少验一类东西，合并照样过。新加的回归放在这一步最后一条钉住的行之后不用改这里；要放在前面就先钉上。
const REGRESSIONS: [&str; 48] = [
    "for script in scripts/package-agent.sh scripts/run-agent-tests.sh scripts/sync-agent-vault.sh scripts/package-preview.sh scripts/run-preview-tests.sh deploy/agent/install-t036-system.sh deploy/agent/upgrade-read-supply.sh.in deploy/agent/enable-m4-policy.sh.in; do",
    "bash -n \"$script\"",
    "done",
    "python3 scripts/test-agent-package.py",
    "python3 scripts/test-project-registry.py",
    "python3 scripts/test-render-agent-upgrade.py",
    "python3 scripts/test-prepare-m4-policy.py",
    "python3 scripts/test-m4-recovery.py",
    "python3 -I -B scripts/test-human-offline.py",
    "python3 scripts/test-check-agent-policy.py",
    "python3 scripts/test-check-policy-revocation.py",
    "python3 -I -B scripts/test-check-backup-preflight.py",
    "python3 scripts/test-preview-package.py",
    "python3 scripts/test-check-agent-http.py",
    "python3 scripts/test-check-agent.py",
    "python3 scripts/test-check-agent-replica.py",
    "python3 scripts/test-sync-agent-vault.py",
    "python3 scripts/test-prepare-agent-sync.py",
    "python3 scripts/test-replica-receiver.py",
    "python3 scripts/test-replica-boundaries.py",
    "python3 scripts/test-check-preview.py",
    "python3 scripts/test-check-preview-exercise.py",
    "python3 scripts/test-package-inputs.py",
    "python3 scripts/test-client-projection-schema.py",
    "python3 scripts/test-client-projection-fetch.py",
    "python3 scripts/test-client-projection-store.py",
    "python3 scripts/test-local-client-core.py",
    "python3 scripts/test-local-client-native.py",
    "python3 scripts/test-local-client-server.py",
    "python3 scripts/test-local-client-contract.py",
    "python3 deploy/blackboard/test_assembly.py",
    "python3 deploy/blackboard/test_freeze.py",
    "python3 deploy/blackboard/test_activation.py",
    "python3 deploy/blackboard/test_sync_tools.py",
    "python3 deploy/blackboard/test_sync_release.py",
    "python3 -I -B deploy/sync-bridge/recovery/test_canonical_snapshot.py",
    "python3 -I -B deploy/sync-bridge/recovery/test_credentials.py",
    "python3 -I -B deploy/sync-bridge/test_pair_production.py",
    "python3 -I -B -S deploy/doorbell/test_units.py",
    "python3 -I -B -S deploy/backup-worker/test_units.py",
    "python3 scripts/tests/test_runner_launch.py",
    "( umask 002; python3 scripts/test-sync-agent-vault.py; python3 scripts/test-replica-receiver.py; python3 scripts/test-replica-boundaries.py )",
    "python3 -I -B -S deploy/alerts/test_monitor.py",
    "cargo --locked build -p studiolo-agent-server",
    "python3 scripts/check-replica-pipeline.py \"${CARGO_TARGET_DIR:-target}/debug/studiolo-agent\"",
    "python3 scripts/check-client-projection-pipeline.py \"${CARGO_TARGET_DIR:-target}/debug/studiolo-agent\"",
    "( umask 002; python3 scripts/check-replica-pipeline.py \"${CARGO_TARGET_DIR:-target}/debug/studiolo-agent\"; python3 scripts/check-client-projection-pipeline.py \"${CARGO_TARGET_DIR:-target}/debug/studiolo-agent\" )",
    "python3 crates/auth-admin/tests/support/cli_process.py \"${CARGO_TARGET_DIR:-target}/debug/studiolo-auth-admin\"",
];

/// 外部心跳 Worker（Node 24.22.2）：安装锁定依赖、测试、生成部署产物，都在它自己的目录里跑。
const WORKER_GATES: [&str; 3] = ["npm ci --ignore-scripts", "npm test", "npm run build"];
const WORKER: &str = "deploy/alerts/worker";

/// 每个有门禁的步骤第一行：环境脚本（补齐沙盒和原 runner 的差别、钉 Node 版本）。
/// 少了它，前端会在沙盒自带的 Node 26 下跑，构建目录也回到跨任务共用的缓存。
fn env_line(cwd: Option<&str>) -> Option<&'static str> {
    match cwd {
        None => Some("source scripts/testbox-ci-env.sh"),
        Some(WEB) => Some("source ../scripts/testbox-ci-env.sh"),
        Some(WORKER) => Some("source ../../../scripts/testbox-ci-env.sh 24.22.2"),
        Some(_) => None,
    }
}

/// 门禁之前可以有的只打印、不改状态的诊断行。
const DIAGNOSTICS: [&str; 5] = [
    "rustc --version",
    "cargo --version",
    "node --version",
    "python3 --version",
    "ulimit -l",
];

/// 每条钉住的门禁和它必须所在的目录（`None` 是仓库根）。
fn pinned() -> impl Iterator<Item = (Option<&'static str>, &'static str)> {
    let root = SHARED
        .into_iter()
        .chain(PRODUCTION)
        .chain(RELEASE)
        .chain(REGRESSIONS)
        .map(|command| (None, command));
    root.chain(FRONTEND.into_iter().map(|command| (Some(WEB), command)))
        .chain(
            WORKER_GATES
                .into_iter()
                .map(|command| (Some(WORKER), command)),
        )
}

/// 这一步的门禁是不是无条件地跑：第一行是对应目录的环境脚本，最后一条门禁之前只有别的门禁和诊断行。
/// 没有门禁的步骤不管。
fn unconditional(step: &Step) -> bool {
    let cwd = step.cwd.as_deref();
    let gate = |line: &str| pinned().any(|(at, command)| at == cwd && command == line);
    let Some(last) = step.run.iter().rposition(|line| gate(line)) else {
        return true;
    };
    step.run.first().map(String::as_str) == env_line(cwd)
        && step.run[1..last]
            .iter()
            .all(|line| gate(line) || DIAGNOSTICS.contains(&line.as_str()))
}

fn label(cwd: Option<&str>, command: &str) -> String {
    format!("{}: {command}", cwd.unwrap_or("."))
}

/// 解析步骤单，每一步顺带算好它的门禁是不是无条件地跑。
fn parse(toml: &str) -> Vec<(Step, bool)> {
    steps(toml)
        .into_iter()
        .map(|step| {
            let unconditional = unconditional(&step);
            (step, unconditional)
        })
        .collect()
}

/// 门禁落在第几步（读得懂、没被削弱、无条件地跑的那一步）；没有就是 `None`。
fn gate_step(steps: &[(Step, bool)], cwd: Option<&str>, command: &str) -> Option<usize> {
    steps.iter().position(|(step, unconditional)| {
        step.suite == SUITE
            && !step.loose
            && *unconditional
            && step.cwd.as_deref() == cwd
            && step.run.iter().any(|line| line == command)
    })
}

/// 步骤单的缺口：没有在 ci 套里指定目录、读得懂、没被削弱、无条件地跑的一步中整行执行的门禁
/// （形如 `目录: 命令`）。
fn suite_gaps(toml: &str) -> Vec<String> {
    let steps = parse(toml);
    pinned()
        .filter(|&(cwd, command)| gate_step(&steps, cwd, command).is_none())
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

/// 红臂只改内存副本：定位目标行（去首尾空白后整行相等，取第一处）后交给 `edit` 改写。
fn mutate(text: &str, target: &str, edit: impl FnOnce(&mut Vec<String>, usize)) -> String {
    mutate_where(text, |lines, at| lines[at].trim() == target, target, edit)
}

/// 同上，但只认写明了这个目录的那一步里的目标行（`npm test` 这类命令前端和 Worker 各有一份）。
fn mutate_in(
    toml: &str,
    cwd: Option<&str>,
    target: &str,
    edit: impl FnOnce(&mut Vec<String>, usize),
) -> String {
    let expected = cwd.map(|cwd| format!("cwd = \"{cwd}\""));
    mutate_where(
        toml,
        |lines, at| {
            let start = header(lines, at);
            let declared = lines[start..at]
                .iter()
                .find(|line| line.trim().starts_with("cwd = "))
                .map(|line| line.trim().to_owned());
            lines[at].trim() == target && declared == expected
        },
        target,
        edit,
    )
}

fn mutate_where(
    text: &str,
    hit: impl Fn(&[String], usize) -> bool,
    target: &str,
    edit: impl FnOnce(&mut Vec<String>, usize),
) -> String {
    let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    let at = (0..lines.len())
        .find(|&at| hit(&lines, at))
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
    (0..=at)
        .rev()
        .find(|&index| lines[index].trim().starts_with("[["))
        .unwrap_or(0)
}

/// 命令所在这一步之后的第一个步骤表头（没有就是文件末尾）。
fn next_header(lines: &[String], at: usize) -> usize {
    (at + 1..lines.len())
        .find(|&index| lines[index].trim().starts_with("[["))
        .unwrap_or(lines.len())
}

/// 同一步里的门禁（它们共进退）。
fn same_step(toml: &str, cwd: Option<&str>, command: &str) -> Vec<String> {
    let steps = parse(toml);
    let step = gate_step(&steps, cwd, command);
    assert!(step.is_some(), "{command} 不在任何一步里");
    pinned()
        .filter(|&(at, other)| gate_step(&steps, at, other) == step)
        .map(|(at, other)| label(at, other))
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
            suite_gaps(&mutate_in(&toml, cwd, command, remove)),
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
    for (cwd, command) in pinned() {
        let off = mutate_in(&toml, cwd, command, |lines, at| {
            let run = above(lines, at, |line| line == "run = '''");
            lines.insert(run + 2, "set +e".to_owned());
        });
        assert_eq!(
            suite_gaps(&off),
            same_step(&toml, cwd, command),
            "{command}"
        );
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

/// 门禁前面插进去的东西让它可能不跑、换了东西跑，或者换了地方跑；开头那行环境脚本删了、改了也一样
/// （复审 A4-F2、A4-F3）。最后一条门禁之后加的行管不着门禁，照旧放行。
#[test]
fn lines_ahead_of_a_gate_and_the_environment_line_are_pinned_too() {
    let (toml, _) = entrypoints();
    let red = same_step(&toml, None, WORKSPACE_TEST);
    for inserted in [
        "exit 0",
        "true && exit 0",
        "if false; then",
        "cd /tmp",
        "PATH=/tmp:$PATH",
        "export CARGO_TARGET_DIR=/tmp",
        "cargo() { :; }",
        "alias cargo=true",
        "trap 'exit 0' EXIT",
        "set -n",
        "source scripts/other.sh",
        "echo ok",
    ] {
        let changed = mutate(&toml, WORKSPACE_TEST, |lines, at| {
            lines.insert(at, inserted.to_owned());
        });
        assert_eq!(suite_gaps(&changed), red, "{inserted}");
    }
    let after = mutate(&toml, WORKSPACE_TEST, |lines, at| {
        lines.insert(at + 1, "echo done".to_owned());
    });
    assert_eq!(suite_gaps(&after), Vec::<String>::new());
    // 每个有门禁的步骤：开头那行环境脚本删掉、换成别的版本或别的目录写法，这一步的门禁都不算数。
    for (cwd, command) in pinned() {
        let line = env_line(cwd).expect("门禁所在目录都有环境脚本");
        let red = same_step(&toml, cwd, command);
        for replacement in [
            None,
            Some(format!("{line} 26.0.0")),
            Some(line.replace("../", "")),
        ] {
            let changed = mutate_in(&toml, cwd, command, |lines, at| {
                let run = above(lines, at, |line| line == "run = '''");
                assert_eq!(lines[run + 1].trim(), line, "{command}");
                match &replacement {
                    Some(text) => lines[run + 1] = text.clone(),
                    None => {
                        lines.remove(run + 1);
                    }
                }
            });
            if replacement.as_deref() == Some(line) {
                continue;
            }
            assert_eq!(suite_gaps(&changed), red, "{command}: {replacement:?}");
        }
    }
}
