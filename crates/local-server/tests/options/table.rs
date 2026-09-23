//! 参数表契约：表是可接受参数的唯一来源；每个带值参数都能解析并落到自己的字段，错误文案逐字不变。

use super::{
    Options, parse,
    table::{Arity, PARAMS},
};
use std::{collections::HashSet, path::Path};

const UNKNOWN: &str = "未知参数；仅接受 --help 中的非秘密参数";
const ORIGIN: &str = "https://studiolo.example.com";
const TEAM: &str = "https://studiolo.cloudflareaccess.com";
const AUDIENCE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const EMAIL: &str = "maxsong@example.com";

/// 本机模式：仓库绑定、Cargo 清单与 Rust 源码白名单只在此模式可用。
const LOCAL: &[&str] = &[
    "--vault",
    "/fixture/vault",
    "--assets",
    "/fixture/web",
    "--label",
    "表驱动副本",
    "--port",
    "4318",
    "--project-repository",
    "项目/虚构=/fixture/repo",
    "--cargo-dependencies",
    "--rust-call-sites",
    "[\"src/lib.rs\"]",
];

/// 旧 Access 公共入口：四个参数必须同时出现。
const LEGACY: &[&str] = &[
    "--vault",
    "copy",
    "--public-origin",
    ORIGIN,
    "--access-team",
    TEAM,
    "--access-audience",
    AUDIENCE,
    "--access-email",
    EMAIL,
];

/// 会话模式下依赖最深的一条命令行：写者、因子、策略、黑板与执行体同时启用。
const SESSION: &[&str] = &[
    "--vault",
    "/fixture/vault",
    "--session-config",
    "/fixture/owner.json",
    "--public-origin",
    ORIGIN,
    "--canonical-writer-data",
    "/fixture/writer",
    "--factor-socket",
    "/fixture/proxy/http.sock",
    "--replacement-seed-ref",
    "next-v2",
    "--policy-data",
    "/fixture/policy",
    "--board-socket",
    "/fixture/board.sock",
    "--board-data",
    "/fixture/board",
    "--board-agent-data",
    "/fixture/agents",
    "--runner-config",
    "/fixture/runner.json",
    "--sync-transport",
    "/fixture/sync-approval.json",
];

const CANONICAL: &[&str] = &[
    "--vault",
    "/fixture/vault",
    "--session-config",
    "/fixture/owner.json",
    "--public-origin",
    ORIGIN,
    "--canonical",
];

fn check(values: &[&str]) -> Result<Option<Options>, String> {
    parse(values.iter().map(|value| (*value).to_owned()))
}

fn error(values: &[&str]) -> Option<String> {
    check(values).err()
}

fn flags() -> impl Iterator<Item = &'static str> {
    PARAMS
        .iter()
        .filter(|(_, arity)| matches!(arity, Arity::Flag(_)))
        .map(|(name, _)| *name)
}

fn valued() -> impl Iterator<Item = &'static str> {
    PARAMS
        .iter()
        .filter(|(_, arity)| matches!(arity, Arity::Value(_)))
        .map(|(name, _)| *name)
}

/// 能通过取值处理的样例值；其余带值参数的取值处理只要求非空且不以 `--` 开头。
fn sample(name: &str) -> &'static str {
    match name {
        "--port" => "4318",
        "--project-repository" => "项目/虚构=/fixture/repo",
        "--rust-call-sites" => "[\"src/lib.rs\"]",
        _ => "/fixture/value",
    }
}

#[test]
fn table_names_are_unique_and_only_three_switches_take_no_value() {
    let names: HashSet<_> = PARAMS.iter().map(|(name, _)| *name).collect();
    assert_eq!(names.len(), PARAMS.len(), "参数表不得出现重复名字");
    assert!(names.iter().all(|name| name.starts_with("--")));
    let mut switches: Vec<_> = flags().collect();
    switches.sort_unstable();
    assert_eq!(switches, ["--canonical", "--cargo-dependencies", "--help"]);
    assert_eq!(valued().count() + switches.len(), PARAMS.len());
}

#[test]
fn every_table_entry_is_accepted_by_a_successful_command_line() {
    let lines: [&[&str]; 5] = [LOCAL, LEGACY, SESSION, CANONICAL, &["-h"]];
    for line in lines {
        assert!(check(line).is_ok(), "{line:?}");
    }
    let covered: HashSet<_> = lines
        .iter()
        .flat_map(|line| line.iter())
        .map(|token| if *token == "-h" { "--help" } else { *token })
        .collect();
    for (name, _) in PARAMS {
        assert!(covered.contains(name), "{name} 缺少成功解析的用例");
    }
    assert!(check(&["-h"]).unwrap().is_none());
    assert!(check(CANONICAL).unwrap().unwrap().canonical);
}

#[test]
fn local_values_land_in_their_own_fields() {
    let options = check(LOCAL).unwrap().unwrap();
    assert_eq!(options.vault, Path::new("/fixture/vault"));
    assert_eq!(options.assets, Path::new("/fixture/web"));
    assert_eq!(options.label, "表驱动副本");
    assert_eq!(options.port, 4318);
    assert_eq!(
        options.project_repository.unwrap(),
        ("项目/虚构".into(), "/fixture/repo".into())
    );
    assert!(options.cargo_dependencies);
    assert_eq!(options.rust_call_sites.unwrap(), ["src/lib.rs"]);
    assert!(options.access.is_none() && options.session.is_none());
}

#[test]
fn legacy_access_values_land_in_the_access_config() {
    let access = format!("{:?}", check(LEGACY).unwrap().unwrap().access.unwrap());
    for value in [ORIGIN, TEAM, AUDIENCE, EMAIL] {
        assert!(access.contains(value), "{value}");
    }
}

#[test]
fn session_writer_factor_board_and_runner_values_land_in_their_own_fields() {
    let options = check(SESSION).unwrap().unwrap();
    let session = options.session.unwrap();
    assert_eq!(session.config, Path::new("/fixture/owner.json"));
    assert_eq!(session.origin, ORIGIN);
    assert_eq!(options.writer_data.unwrap(), Path::new("/fixture/writer"));
    let factor = options.factor.unwrap();
    assert_eq!(factor.socket, Path::new("/fixture/proxy/http.sock"));
    assert_eq!(factor.replacement_seed_ref.as_deref(), Some("next-v2"));
    assert_eq!(options.policy_data.unwrap(), Path::new("/fixture/policy"));
    let board = options.board.unwrap();
    assert_eq!(board.socket, Path::new("/fixture/board.sock"));
    assert_eq!(board.data, Path::new("/fixture/board"));
    assert_eq!(board.agent_data, Path::new("/fixture/agents"));
    let runner = options.runner_config.unwrap();
    assert_eq!(runner, Path::new("/fixture/runner.json"));
    assert_eq!(options.port, 4317);
    assert!(!options.canonical && options.access.is_none());
}

#[test]
fn every_value_parameter_reports_a_missing_value_verbatim() {
    for name in valued() {
        let expected = format!("参数 {name} 缺少值");
        for tail in [&[][..], &[""], &["-h"], &["--vault", "copy"]] {
            let values = [&[name][..], tail].concat();
            assert_eq!(error(&values).as_deref(), Some(expected.as_str()));
        }
    }
}

#[test]
fn every_parameter_reports_a_duplicate_verbatim() {
    for (name, arity) in PARAMS {
        let once = match arity {
            Arity::Flag(_) => vec![*name],
            Arity::Value(_) => vec![*name, sample(name)],
        };
        let expected = format!("参数 {name} 不得重复");
        assert_eq!(error(&once.repeat(2)), Some(expected), "{name}");
    }
    assert_eq!(
        error(&["-h", "--help"]).as_deref(),
        Some("参数 --help 不得重复")
    );
}

#[test]
fn unknown_arguments_keep_the_verbatim_message_and_switches_never_take_values() {
    for value in [
        "--unknown",
        "copy",
        "--port=4318",
        "--VAULT",
        "-v",
        "",
        "--",
        "-h1",
    ] {
        assert_eq!(error(&[value]).as_deref(), Some(UNKNOWN), "{value}");
        let after = ["--vault", "copy", value];
        assert_eq!(error(&after).as_deref(), Some(UNKNOWN), "{value}");
    }
    for switch in flags() {
        let values = [switch, "/fixture/value"];
        assert_eq!(error(&values).as_deref(), Some(UNKNOWN), "{switch}");
    }
}

#[test]
fn arguments_are_checked_in_order_so_the_first_error_wins() {
    let port = Some("端口必须为 1–65535");
    assert_eq!(error(&["--port", "0", "--unknown"]).as_deref(), port);
    assert_eq!(
        error(&["--unknown", "--port", "0"]).as_deref(),
        Some(UNKNOWN)
    );
    assert_eq!(
        error(&["--port", "4318", "--port", "0"]).as_deref(),
        Some("参数 --port 不得重复")
    );
    assert_eq!(
        error(&["--vault", "--port", "4318"]).as_deref(),
        Some("参数 --vault 缺少值")
    );
}
