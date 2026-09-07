//! 写者模式只从显式参数启用，解析阶段不访问任何目录。

use super::parse;

fn arguments() -> Vec<String> {
    [
        "--vault",
        "/fixture/vault",
        "--assets",
        "/fixture/assets",
        "--session-config",
        "/fixture/config.json",
        "--public-origin",
        "https://studiolo.example.com",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[test]
fn writer_mode_is_explicit_and_does_not_upgrade_the_old_canonical_flag() {
    let original = parse(arguments().into_iter()).unwrap().unwrap();
    assert!(original.writer_data.is_none());
    let mut old = arguments();
    old.push("--canonical".into());
    let old = parse(old.into_iter()).unwrap().unwrap();
    assert!(old.canonical);
    assert!(old.writer_data.is_none());
    let mut new = arguments();
    new.extend(["--canonical-writer-data".into(), "/fixture/data".into()]);
    let new = parse(new.into_iter()).unwrap().unwrap();
    assert!(!new.canonical);
    assert_eq!(
        new.writer_data.unwrap(),
        std::path::Path::new("/fixture/data")
    );
}

#[test]
fn incomplete_relative_duplicate_and_mixed_writer_modes_are_rejected() {
    for extra in [
        vec!["--canonical-writer-data"],
        vec!["--canonical-writer-data", ""],
        vec!["--canonical-writer-data", "relative"],
        vec!["--canonical-writer-data", "/fixture/data", "--canonical"],
        vec![
            "--canonical-writer-data",
            "/fixture/data",
            "--canonical-writer-data",
            "/other",
        ],
    ] {
        let mut values = arguments();
        values.extend(extra.into_iter().map(str::to_owned));
        assert!(parse(values.into_iter()).is_err());
    }
    assert!(
        parse(
            [
                "--vault",
                "/fixture/vault",
                "--canonical-writer-data",
                "/fixture/data"
            ]
            .into_iter()
            .map(str::to_owned)
        )
        .is_err()
    );
}

#[test]
fn writer_mode_accepts_the_existing_explicit_unix_factor_transport() {
    let mut values = arguments();
    values.extend(
        [
            "--canonical-writer-data",
            "/fixture/data",
            "--factor-socket",
            "/fixture/proxy/http.sock",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    let options = parse(values.into_iter()).unwrap().unwrap();
    assert!(options.writer_data.is_some());
    assert!(options.factor.is_some());
}
