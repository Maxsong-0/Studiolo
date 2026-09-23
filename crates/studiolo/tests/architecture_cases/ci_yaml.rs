//! ci.yml 的块状 YAML 子集解析：各步骤执行哪些命令、门禁会不会被绕过，以及顶层 `on:` 触发条件。
//! 只认本仓库用到的写法；读不懂的一律往坏处算——不产生命令、步骤算松动、触发条件算缺——
//! 守卫因此报缺而不是放行。

/// `jobs:` 下作业名的缩进，与作业自身键（`steps:`、`if:` 等）的缩进。
pub(super) const JOB_INDENT: usize = 2;
const JOB_KEY_INDENT: usize = 4;

/// ci.yml 的一个步骤。
#[derive(Default)]
pub(super) struct Step {
    pub(super) job: String,
    /// 门禁原样留着也可能不跑、跑挂了也不拦：步骤或所在作业带 `if:`、`continue-on-error:`
    /// （值不是 `false`）或 `<<` 合并键（可能带进 `if:`，看不见内容就按带条件算）；步骤自身、
    /// 所在作业或全局 `defaults.run` 的 `shell` 不是恰好 `bash`（`sh`、`bash {0}`、`pwsh` 都
    /// 不保证 errexit）；或 run 块里有一行 `set +e` 之类关掉 errexit 的写法。
    pub(super) loose: bool,
    /// `run` 的命令：解析时是物理行，收尾后拼成逻辑行；折叠块 `>` 不计入。
    pub(super) run: Vec<String>,
}

pub(super) fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// 去掉首尾空白与引号后的标量。
fn scalar(text: &str) -> &str {
    text.trim().trim_matches(['"', '\'']).trim()
}

/// 键值行拆成（去引号的键，去首尾空白的值）；没有冒号就不是键值行。
fn key_value(text: &str) -> Option<(&str, &str)> {
    text.split_once(':')
        .map(|(key, value)| (scalar(key), value.trim()))
}

/// 把以 `\` 续行的物理行（已去首尾空白）拼成 shell 实际执行的逻辑行。
pub(super) fn logical_lines<'a>(physical: impl IntoIterator<Item = &'a str>) -> Vec<String> {
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

/// 这一行关掉了 errexit：按 `;`、`&`、`|`、括号与花括号切成简单命令，任一段（可带 `then`／`do`／
/// `else`／`builtin`／`command` 前缀）是 `set`，且参数里有含 `e` 的 `+` 开关或 `+o errexit`——
/// `set +e`、`true; set +e`、`{ set +e; }`、`set -u +e`、`set +o errexit` 都算。此后失败的命令
/// 不再中止脚本，退出码只看最后一条。不解析引号：引号里恰好出现「; set +e 词」会误判为红，宁严勿宽。
pub(super) fn disables_errexit(line: &str) -> bool {
    let separators = |c: char| matches!(c, ';' | '&' | '|' | '(' | ')' | '{' | '}');
    line.split(separators).any(|segment| {
        let mut words = segment
            .split_whitespace()
            .skip_while(|word| matches!(*word, "then" | "do" | "else" | "builtin" | "command"));
        if words.next() != Some("set") {
            return false;
        }
        let args = words.collect::<Vec<_>>();
        args.iter()
            .enumerate()
            .any(|(at, arg)| match arg.strip_prefix('+') {
                Some("o") => args.get(at + 1) == Some(&"errexit"),
                Some(flags) => {
                    flags.contains('e') && flags.chars().all(|c| c.is_ascii_alphabetic())
                }
                None => false,
            })
    })
}

#[test]
fn errexit_switch_offs_follow_the_documented_pattern() {
    let off = "set +e|  set +eu|set +xe;|set  +o\terrexit|set +o errexit;|true; set +e|{ set +e; }\
               |set -u +e|if x; then set +e; fi|a && set +e|builtin set +e|set +eo pipefail";
    let kept = "set -e|set -euo pipefail|set +x|set +o pipefail|set +o errexits|set+e|echo set +e\
                |echo 'set +e'|grep -q x f || true|[ -n \"$x\" ] && set -e";
    for (lines, expected) in [(off, true), (kept, false)] {
        for line in lines.split('|').map(str::trim) {
            assert_eq!(disables_errexit(line), expected, "{line}");
        }
    }
}

/// `defaults:` 块里的一行仍让 run 用 bash：只认 `run:`、`shell: bash` 与 `working-directory:`；
/// 合并键、流式映射、列表项等读不懂的写法都按换了 shell 算。
fn keeps_bash(text: &str) -> bool {
    match key_value(text) {
        Some(("run", "") | ("working-directory", _)) => true,
        Some(("shell", value)) => scalar(value) == "bash",
        _ => false,
    }
}

/// 只解析本仓库 ci.yml 用到的块状 YAML 子集；读不懂的写法不产生命令，守卫因此报缺而不是放行。
pub(super) fn workflow_steps(yaml: &str) -> Vec<Step> {
    let (mut steps, mut loose_jobs) = (Vec::<Step>::new(), Vec::<String>::new());
    let (mut in_jobs, mut in_steps, mut job) = (false, false, String::new());
    // 当前作业 steps 列表项的缩进；更深处的 `- ` 属于步骤内部，不是新步骤。
    let mut item: Option<usize> = None;
    // 正在跳过的块标量：（所属键的缩进，是否为步骤 run 的字面块）。
    let mut block: Option<(usize, bool)> = None;
    // 正在读的 `defaults:` 块的键缩进（0 即全局）；全局的换了 shell，所有步骤都算松动。
    let (mut defaults, mut all_loose) = (None::<usize>, false);
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
        if let Some(owner) = defaults.filter(|&owner| depth > owner) {
            match (keeps_bash(text), owner) {
                (true, _) => {}
                (false, 0) => all_loose = true,
                (false, _) => loose_jobs.push(job.clone()),
            }
            continue;
        }
        defaults = None;
        if depth == 0 {
            in_jobs = text == "jobs:";
            let (key, value) = key_value(text).unwrap_or((text, ""));
            defaults = (key == "defaults").then_some(0);
            // 合并键可能带进 defaults，行内写法读不懂：都按全局换了 shell 算。
            all_loose |= key == "<<" || (defaults.is_some() && !value.is_empty());
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
        let Some((key, value)) = key_value(entry) else {
            continue;
        };
        let loose = matches!(key, "if" | "<<")
            || (key == "continue-on-error" && value != "false")
            || (key == "shell" && scalar(value) != "bash");
        let step_key = item.is_some_and(|level| key_depth == level + 2);
        if value.starts_with(['|', '>']) {
            block = Some((
                key_depth,
                step_key && key == "run" && value.starts_with('|'),
            ));
        }
        if key_depth == JOB_KEY_INDENT {
            (in_steps, item) = (key == "steps", None);
            defaults = (key == "defaults").then_some(JOB_KEY_INDENT);
            if loose || (defaults.is_some() && !value.is_empty()) {
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
        let errexit_off = step.run.iter().map(String::as_str).any(disables_errexit);
        step.loose |= all_loose || errexit_off || loose_jobs.contains(&step.job);
        step.run = logical_lines(step.run.iter().map(String::as_str));
    }
    steps
}

/// 块状映射的一项：键（去引号）、行内值，与下属的非空非注释行（保留原缩进）。
struct Entry<'a> {
    key: &'a str,
    value: &'a str,
    body: Vec<&'a str>,
}

/// 把同一层块状映射切成项；列表项、缺冒号或缩进不齐都读不懂（`None`）。
fn entries<'a>(lines: &[&'a str]) -> Option<Vec<Entry<'a>>> {
    let level = lines.first().map_or(0, |line| indent(line));
    let mut items = Vec::<Entry<'a>>::new();
    for &line in lines {
        if indent(line) > level {
            items.last_mut()?.body.push(line);
            continue;
        }
        let text = line.trim();
        let (key, value) =
            key_value(text).filter(|_| indent(line) == level && !text.starts_with('-'))?;
        items.push(Entry {
            key,
            value,
            body: Vec::new(),
        });
    }
    Some(items)
}

/// `[a, "b"]` 或逐行 `- a` 写成的列表，各项去引号；其余写法或有空项都读不懂（`None`）。
fn list<'a>(entry: &Entry<'a>) -> Option<Vec<&'a str>> {
    let items = match (entry.value.strip_prefix('['), entry.body.is_empty()) {
        (Some(inner), true) => inner.strip_suffix(']')?.split(',').map(scalar).collect(),
        (None, _) if entry.value.is_empty() => entry
            .body
            .iter()
            .map(|&line| line.trim().strip_prefix("- ").map(scalar))
            .collect::<Option<Vec<_>>>()?,
        _ => return None,
    };
    (!items.contains(&"")).then_some(items)
}

/// 恰好一处、值留空的 `key` 项下属的块状映射；缺失、重复或读不懂都是 `None`。
fn only<'a>(items: &[Entry<'a>], key: &str) -> Option<Vec<Entry<'a>>> {
    let mut found = items.iter().filter(|item| item.key == key);
    match (found.next(), found.next()) {
        (Some(item), None) if item.value.is_empty() => entries(&item.body),
        _ => None,
    }
}

/// `on:` 触发条件的缺口，每条一句中文：push 须恰只对 main 触发，pull_request 不许被分支、
/// 路径或事件类型过滤掉。只认顶层唯一的块状 `on:`（键可带引号）；`true:`、流式写法等读不懂，
/// 按两个事件都没配算。
pub(super) fn trigger_gaps(yaml: &str) -> Vec<String> {
    let lines = yaml
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
        .collect::<Vec<_>>();
    let Some(events) = only(&entries(&lines).unwrap_or_default(), "on") else {
        return vec!["on: 须是顶层唯一的块状映射，否则按没配触发算".to_owned()];
    };
    let mut gaps = Vec::new();
    for (event, allowed) in [("push", "branches"), ("pull_request", "types")] {
        let Some(filters) = only(&events, event) else {
            gaps.push(format!("on.{event} 须恰好一处，留空或写成块状映射"));
            continue;
        };
        let mut values = Vec::new();
        for filter in &filters {
            let key = filter.key;
            if key == allowed {
                values.push(list(filter));
            } else {
                gaps.push(format!("on.{event} 下不许有 {key}：会放宽或过滤触发"));
            }
        }
        let gap = match (event, &values[..]) {
            ("push", [Some(branches)]) if branches[..] == ["main"] => continue,
            ("push", _) => "branches 须恰为 [main] 且只写一处，否则会对别的分支触发".to_owned(),
            (_, []) => continue,
            (_, [Some(types)]) => {
                let missing = ["opened", "synchronize", "reopened"]
                    .into_iter()
                    .filter(|kind| !types.contains(kind))
                    .collect::<Vec<_>>();
                if missing.is_empty() {
                    continue;
                }
                format!("types 缺 {}：这类 PR 更新不跑 CI", missing.join("、"))
            }
            _ => "types 须只写一处且写成列表".to_owned(),
        };
        gaps.push(format!("on.{event}.{gap}"));
    }
    gaps
}
