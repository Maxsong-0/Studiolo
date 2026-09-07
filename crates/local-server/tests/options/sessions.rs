//! 新参数仍在纯解析Seam验收，不访问配置文件或秘密环境。

use super::parse;

fn check(values: &[&str]) -> Result<Option<super::Options>, String> {
    parse(values.iter().map(|value| (*value).to_owned()))
}

const BASE: &[&str] = &[
    "--vault",
    "/fixture/vault",
    "--assets",
    "/fixture/web",
    "--session-config",
    "/fixture/config/owner.json",
    "--public-origin",
    "https://studiolo.example.com",
];

#[test]
fn session_configuration_is_explicit_and_excludes_legacy_access() {
    let options = check(BASE).unwrap().unwrap();
    assert!(options.access.is_none());
    let session = options.session.unwrap();
    assert_eq!(
        session.config.to_str().unwrap(),
        "/fixture/config/owner.json"
    );
    assert_eq!(session.origin, "https://studiolo.example.com");
    for flag in ["--access-team", "--access-audience", "--access-email"] {
        let mut values = BASE.to_vec();
        values.extend([flag, "fictional"]);
        assert!(check(&values).is_err());
    }
}

#[test]
fn session_flags_reject_missing_relative_repeated_and_help_ambiguity() {
    for values in [
        vec!["--session-config"],
        vec!["--session-config", ""],
        vec!["--session-config", "/fixture.json"],
        vec!["--help", "--session-config", "/fixture.json"],
        vec![
            "--session-config",
            "relative.json",
            "--public-origin",
            "https://example.com",
        ],
    ] {
        assert!(check(&values).is_err());
    }
    let mut duplicate = BASE.to_vec();
    duplicate.extend(["--session-config", "/another.json"]);
    assert!(check(&duplicate).is_err());
}

#[test]
fn unknown_argument_contents_are_never_echoed() {
    let error = check(&["DO_NOT_ECHO_SYNTHETIC_VALUE"]).err().unwrap();
    assert!(!error.contains("DO_NOT_ECHO_SYNTHETIC_VALUE"));
}

#[test]
fn canonical_requires_explicit_session_mode_and_is_never_inferred() {
    assert!(!check(BASE).unwrap().unwrap().canonical);
    let mut values = BASE.to_vec();
    values.push("--canonical");
    assert!(check(&values).unwrap().unwrap().canonical);
    values.push("--canonical");
    assert!(check(&values).is_err());
    assert!(check(&["--vault", "/fixture/vault", "--canonical"]).is_err());
}

#[test]
fn factor_mode_is_explicit_and_never_adds_a_tcp_listener() {
    assert!(check(BASE).unwrap().unwrap().factor.is_none());
    let mut values = BASE.to_vec();
    values.extend([
        "--factor-socket",
        "/fixture/proxy/http.sock",
        "--replacement-seed-ref",
        "next-v2",
        "--canonical",
    ]);
    let options = check(&values).unwrap().unwrap();
    assert!(options.canonical);
    let factor = options.factor.unwrap();
    assert_eq!(factor.socket.to_str().unwrap(), "/fixture/proxy/http.sock");
    assert_eq!(factor.replacement_seed_ref.as_deref(), Some("next-v2"));
    for extra in [
        ["--port", "4317"],
        ["--factor-socket", "/duplicate.sock"],
        ["--replacement-seed-ref", "duplicate"],
    ] {
        let mut bad = values.clone();
        bad.extend(extra);
        assert!(check(&bad).is_err());
    }
}

#[test]
fn incomplete_factor_options_cannot_silently_become_a_legacy_mode() {
    for extra in [
        vec!["--replacement-seed-ref", "next"],
        vec!["--factor-socket", "relative.sock"],
        vec!["--factor-socket"],
        vec!["--factor-socket", ""],
    ] {
        let mut bad = BASE.to_vec();
        bad.extend(extra);
        assert!(check(&bad).is_err());
    }
    assert!(
        check(&[
            "--vault",
            "/fixture/vault",
            "--factor-socket",
            "/fixture/proxy/http.sock"
        ])
        .is_err()
    );
}
