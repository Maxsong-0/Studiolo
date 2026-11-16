//! `.testbox.toml` 步骤单的 TOML 子集解析：每一步属于哪一套、在哪个目录、执行哪些命令、门禁会不会被绕过。
//! 只认本仓库步骤单用到的写法（双引号字符串不带转义、run 用独占一行的 `'''` 开合、键后不跟注释，
//! 表头只有 `[suites.<套>]` 和 `[[suites.<套>.steps]]`）；读不懂的一律往坏处算——这一步算松动、
//! 命令都不算数——守卫因此报缺而不是放行。别的表头（如 `[suites.ci.steps.env]`）在 TOML 里会把键
//! 挂到刚结束的那一步上，所以那一步也算松动。
//! 测试箱按 `bash -eo pipefail -c` 执行每一步的 run，所以 errexit 默认开着；run 里关掉它的整步算松动。

/// 步骤单里的一步。
pub(super) struct Step {
    /// 所属的套（`[[suites.<套>.steps]]`）。
    pub(super) suite: String,
    /// 工作目录（相对仓库根）；没写是仓库根。
    pub(super) cwd: Option<String>,
    /// 读不懂、是后台服务、或 run 里关掉了 errexit：这一步的命令都不算数。
    pub(super) loose: bool,
    /// shell 实际执行的逻辑行（续行已拼好、首尾空白已去）。
    pub(super) run: Vec<String>,
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

/// 双引号字符串的内容；带转义、缺引号或后面还跟着东西都读不懂（`None`）。
fn quoted(value: &str) -> Option<&str> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains(['"', '\\'])).then_some(inner)
}

/// 不影响门禁跑不跑、跑挂了拦不拦的键。
const HARMLESS: [&str; 5] = ["title", "needs", "timeout_sec", "junit", "kind"];

/// 套名只认小写字母、数字、`_`、`-`：带点、引号、空白的表头在 TOML 里可能指向别处。
fn simple(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
}

/// 解析全部步骤（各套都在，调用方按套筛）。
pub(super) fn steps(toml: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut current: Option<Step> = None;
    let mut lines = toml.lines();
    while let Some(raw) = lines.next() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let step_suite = line
                .strip_prefix("[[suites.")
                .and_then(|rest| rest.strip_suffix(".steps]]"))
                .filter(|suite| simple(suite));
            let suite_table = line
                .strip_prefix("[suites.")
                .and_then(|rest| rest.strip_suffix(']'))
                .is_some_and(simple);
            // 别的表头（如 `[suites.ci.steps.env]`）在 TOML 里可能把键挂到刚结束的这一步上：往坏处算。
            if step_suite.is_none()
                && !suite_table
                && let Some(step) = current.as_mut()
            {
                step.loose = true;
            }
            steps.extend(current.take());
            current = step_suite.map(|suite| Step {
                suite: suite.to_owned(),
                cwd: None,
                loose: false,
                run: Vec::new(),
            });
            continue;
        }
        // 套本身的键（如 lanes）不影响哪一步跑什么。
        let Some(step) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            step.loose = true;
            continue;
        };
        match (key.trim(), value.trim()) {
            ("run", "'''") => {
                let mut body = Vec::new();
                let mut closed = false;
                for raw in lines.by_ref() {
                    if raw.trim() == "'''" {
                        closed = true;
                        break;
                    }
                    body.push(raw.trim());
                }
                // 正文里再出现 `'''` 时，TOML 在那里就结束了字符串，后面的行不是这一步的命令。
                let early_end = body.iter().any(|line| line.contains("'''"));
                if !closed
                    || early_end
                    || !step.run.is_empty()
                    || body.iter().any(|line| disables_errexit(line))
                {
                    step.loose = true;
                }
                step.run = logical_lines(body);
            }
            ("cwd", value) if step.cwd.is_none() => match quoted(value) {
                Some(cwd) => step.cwd = Some(cwd.to_owned()),
                None => step.loose = true,
            },
            ("name", value) if quoted(value).is_some() => {}
            ("service", "false") => {}
            (key, _) if HARMLESS.contains(&key) => {}
            _ => step.loose = true,
        }
    }
    steps.extend(current.take());
    steps
}

#[test]
fn unreadable_or_weakened_shapes_make_the_step_loose() {
    let step = |body: &str| {
        let parsed = steps(&format!("[[suites.ci.steps]]\nname = \"a\"\n{body}"));
        assert_eq!(parsed.len(), 1);
        (parsed[0].loose, parsed[0].run.clone())
    };
    let ok = step("run = '''\nsource x\ncargo test\\\n  --workspace\n'''\ntimeout_sec = 9");
    assert_eq!(
        ok,
        (
            false,
            vec!["source x".into(), "cargo test --workspace".into()]
        )
    );
    for body in [
        "run = \"cargo test\"",
        "run = '''cargo test\n'''",
        "run = '''\ncargo test",
        "run = '''\nset +e\ncargo test\n'''",
        "run = '''\ncargo test\n'''\nservice = true",
        "run = '''\ncargo test\n'''\nenv = { A = \"1\" }",
        "run = '''\ncargo test\n'''\ncwd = \"we\\\"b\"",
        "run = '''\ncargo test\n'''\nrun = '''\ncargo test\n'''",
        "run = '''\ncargo test\n'''\nneeds = [\n  \"x\",\n]",
        "name = \"b\" # 注释\nrun = '''\ncargo test\n'''",
        "run = '''\ntrue'''\nx = '''\ncargo test\n'''",
        "run = '''\ncargo test\n'''\n[suites.ci.steps.env]\nA = \"1\"",
        "run = '''\ncargo test\n'''\n[ suites.ci.steps.env ]\nA = \"1\"",
        "run = '''\ncargo test\n'''\n[[suites.ci.steps.nested]]\nA = \"1\"",
    ] {
        assert!(step(body).0, "{body}");
    }
    // 下一步和别的套的表头不连累上一步；套名不规整的表头读不懂：上一步算松动，它带的那一步不算数。
    let parsed = steps(
        "[suites.ci]\nlanes = 1\n[[suites.ci.steps]]\nname = \"a\"\nrun = '''\ncargo test\n'''\n\
         [suites.other]\n[[suites.ci.steps]]\nname = \"b\"\nrun = '''\ncargo test\n'''\n\
         [[suites.\"ci\".steps]]\nname = \"c\"\nrun = '''\ncargo test\n'''",
    );
    let shape = parsed
        .iter()
        .map(|step| (step.suite.as_str(), step.loose))
        .collect::<Vec<_>>();
    assert_eq!(shape, [("ci", false), ("ci", true)]);
}
