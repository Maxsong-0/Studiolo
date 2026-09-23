//! CI 入口逐行钉住门禁命令：整条逻辑命令行相等才算数，子串命中不算（旧写法的重言式：
//! `cargo --locked test` 会被路径安全那一行满足）。ci.yml 里的命令还必须落在指定作业、
//! 所在步骤与作业都不带 `if:`，也不 `continue-on-error`——否则门禁可以原样留着却不拦合并。

use crate::support::workspace_root;
use std::fs;

const WORKFLOW: &str = ".github/workflows/ci.yml";
const LINUX: &str = "scripts/test-linux.sh";
const WORKSPACE_TEST: &str = "cargo --locked test --workspace";

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

/// `jobs:` 下作业名的缩进，与作业自身键（`steps:`、`if:` 等）的缩进。
const JOB_INDENT: usize = 2;
const JOB_KEY_INDENT: usize = 4;

/// ci.yml 的一个步骤。
#[derive(Default)]
struct Step {
    job: String,
    /// 步骤或所在作业带 `if:`、`continue-on-error:`（值不是 `false`），或 `<<` 合并键
    /// （可能带进 `if:`，看不见内容就按带条件算）。
    loose: bool,
    /// `run` 的命令：解析时是物理行，收尾后拼成逻辑行；折叠块 `>` 不计入。
    run: Vec<String>,
}

fn pinned() -> impl Iterator<Item = (&'static str, &'static str)> {
    let rust = SHARED
        .into_iter()
        .chain(PRODUCTION)
        .map(|command| ("rust", command));
    rust.chain(FRONTEND.into_iter().map(|command| ("frontend", command)))
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// 把以 `\` 续行的物理行（已去首尾空白）拼成 shell 实际执行的逻辑行。
fn logical_lines<'a>(physical: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let (mut lines, mut pending) = (Vec::new(), String::new());
    for line in physical {
        pending.push_str(line.strip_suffix('\\').unwrap_or(line));
        if line.ends_with('\\') {
            pending.push(' ');
        } else {
            lines.push(std::mem::take(&mut pending).trim().to_owned());
        }
    }
    if !pending.trim().is_empty() {
        lines.push(pending.trim().to_owned());
    }
    lines
}

/// 只解析本仓库 ci.yml 用到的块状 YAML 子集；读不懂的写法不产生命令，守卫因此报缺而不是放行。
fn workflow_steps(yaml: &str) -> Vec<Step> {
    let (mut steps, mut loose_jobs) = (Vec::<Step>::new(), Vec::<String>::new());
    let (mut in_jobs, mut in_steps, mut job) = (false, false, String::new());
    // 当前作业 steps 列表项的缩进；更深处的 `- ` 属于步骤内部，不是新步骤。
    let mut item: Option<usize> = None;
    // 正在跳过的块标量：（所属键的缩进，是否为步骤 run 的字面块）。
    let mut block: Option<(usize, bool)> = None;
    for raw in yaml.lines() {
        let (text, depth) = (raw.trim(), indent(raw));
        if let Some((owner, literal)) = block {
            if text.is_empty() || depth > owner {
                if let (true, Some(step)) = (literal, steps.last_mut()) {
                    step.run.push(text.to_owned());
                }
                continue;
            }
            block = None;
        }
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        if depth == 0 {
            in_jobs = text == "jobs:";
            continue;
        }
        if !in_jobs {
            continue;
        }
        if depth == JOB_INDENT {
            job = text.trim_end_matches(':').to_owned();
            (in_steps, item) = (false, None);
            continue;
        }
        let (key_depth, entry) = match text.strip_prefix("- ") {
            Some(rest) if in_steps && item.is_none_or(|level| level == depth) => {
                item = Some(depth);
                steps.push(Step {
                    job: job.clone(),
                    ..Step::default()
                });
                (depth + 2, rest)
            }
            _ => (depth, text),
        };
        let Some((key, value)) = entry.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim().trim_matches(['"', '\'']), value.trim());
        let loose = matches!(key, "if" | "<<") || (key == "continue-on-error" && value != "false");
        let step_key = item.is_some_and(|level| key_depth == level + 2);
        if value.starts_with(['|', '>']) {
            block = Some((
                key_depth,
                step_key && key == "run" && value.starts_with('|'),
            ));
        }
        if key_depth == JOB_KEY_INDENT {
            (in_steps, item) = (key == "steps", None);
            if loose {
                loose_jobs.push(job.clone());
            }
        } else if let (true, Some(step)) = (step_key, steps.last_mut()) {
            step.loose |= loose;
            if key == "run" && !value.starts_with(['|', '>']) {
                step.run.push(value.to_owned());
            }
        }
    }
    for step in &mut steps {
        step.loose |= loose_jobs.contains(&step.job);
        step.run = logical_lines(step.run.iter().map(String::as_str));
    }
    steps
}

/// ci.yml 里没有在指定作业中被无条件、整行执行的门禁，形如 `作业: 命令`。
fn workflow_gaps(yaml: &str) -> Vec<String> {
    let steps = workflow_steps(yaml);
    pinned()
        .filter(|&(job, command)| {
            !steps.iter().any(|step| {
                step.job == job && !step.loose && step.run.iter().any(|line| line == command)
            })
        })
        .map(|(job, command)| format!("{job}: {command}"))
        .collect()
}

/// test-linux.sh 默认回路里缺少的共享 Rust 门禁（按整条逻辑命令行比对）。
fn script_gaps(script: &str) -> Vec<&'static str> {
    let lines = logical_lines(script.lines().map(str::trim));
    SHARED
        .into_iter()
        .filter(|command| !lines.iter().any(|line| line == command))
        .collect()
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

/// 红臂只改内存副本：定位命令所在行（行内 `run:` 与块内行都认）后交给 `edit` 改写。
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

fn with_key(lines: &mut Vec<String>, owner: usize, key: &str) {
    let pad = " ".repeat(indent(&lines[owner]) + 2);
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
    // 步骤带条件或容错：命令原样还在，但门禁可能不跑、跑挂了也不拦。
    for key in ["if: false", "continue-on-error: true", "'if': ${{ false }}"] {
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
