//! 显式白名单、预算及模式约束必须在任何文件读取之前完成。

use super::parse;

fn options(extra: &[&str]) -> Result<Option<super::Options>, String> {
    parse(
        ["--vault", "fixture"]
            .into_iter()
            .chain(extra.iter().copied())
            .map(str::to_owned),
    )
}

#[test]
fn rust_sources_are_disabled_by_default_and_independent_of_cargo() {
    assert!(options(&[]).unwrap().unwrap().rust_call_sites.is_none());
    let binding = ["--project-repository", "项目/虚构=/tmp/repo"];
    assert!(
        options(&binding)
            .unwrap()
            .unwrap()
            .rust_call_sites
            .is_none()
    );
    assert!(options(&["--rust-call-sites", "[\"src/lib.rs\"]"]).is_err());
    for cargo in [false, true] {
        let mut args = vec![
            binding[0],
            binding[1],
            "--rust-call-sites",
            "[\"src/z.rs\",\"src/a.rs\"]",
        ];
        if cargo {
            args.push("--cargo-dependencies");
        }
        let result = options(&args).unwrap().unwrap();
        assert_eq!(result.rust_call_sites.unwrap(), ["src/a.rs", "src/z.rs"]);
        assert_eq!(result.cargo_dependencies, cargo);
    }
}

#[test]
fn rust_source_json_is_closed_bounded_and_does_not_echo_input() {
    let large = format!("[\"{}\"]", "PRIVATE".repeat(5000));
    for value in [
        "null",
        "{}",
        "[]",
        "[1]",
        "[\"PRIVATE\"]",
        "[\"../PRIVATE.rs\"]",
        "[\"a.rs\",\"a.rs\"]",
        "[\"a.rs\"] garbage",
        &large,
    ] {
        let error = options(&[
            "--project-repository",
            "项目/虚构=/tmp/repo",
            "--rust-call-sites",
            value,
        ])
        .err()
        .unwrap();
        assert!(!error.contains("PRIVATE"));
        assert!(!error.contains(value));
    }
    let exact = format!("[\"a.rs\"]{}", " ".repeat(32 * 1024 - 8));
    assert_eq!(exact.len(), 32 * 1024);
    assert!(
        options(&[
            "--project-repository",
            "项目/虚构=/tmp/repo",
            "--rust-call-sites",
            &exact
        ])
        .is_ok()
    );
}

#[test]
fn rust_sources_reject_duplicate_flags_missing_values_and_session_modes() {
    for extra in [
        vec!["--rust-call-sites"],
        vec![
            "--rust-call-sites",
            "[\"a.rs\"]",
            "--rust-call-sites",
            "[\"b.rs\"]",
        ],
        vec!["--rust-call-sites", "[\"a.rs\"]", "--canonical"],
        vec![
            "--rust-call-sites",
            "[\"a.rs\"]",
            "--session-config",
            "/tmp/owner.json",
            "--public-origin",
            "https://studiolo.example.com",
        ],
    ] {
        let args: Vec<_> = ["--project-repository", "项目/虚构=/tmp/repo"]
            .into_iter()
            .chain(extra)
            .collect();
        assert!(options(&args).is_err());
    }
    assert!(
        options(&["--help", "--rust-call-sites", "[\"does-not-exist.rs\"]"])
            .unwrap()
            .is_none()
    );
}
