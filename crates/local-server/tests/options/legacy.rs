use super::parse;
use std::path::PathBuf;

const PUBLIC: [(&str, &str); 4] = [
    ("--public-origin", "https://studiolo.example.com"),
    ("--access-team", "https://studiolo.cloudflareaccess.com"),
    (
        "--access-audience",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ),
    ("--access-email", "maxsong@example.com"),
];

fn args(values: &[&str]) -> std::vec::IntoIter<String> {
    values
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>()
        .into_iter()
}

#[test]
fn local_defaults_require_only_an_explicit_vault() {
    let options = parse(args(&["--vault", "examples/m1-vault"]))
        .unwrap()
        .unwrap();
    assert_eq!(options.vault, PathBuf::from("examples/m1-vault"));
    assert_eq!(options.assets, PathBuf::from("web/dist"));
    assert_eq!(options.label, "只读副本");
    assert_eq!(options.port, 4317);
    assert!(options.access.is_none());
    assert!(parse(args(&[])).is_err());
}

#[test]
fn explicit_local_values_are_preserved() {
    let options = parse(args(&[
        "--label",
        "服务器示例库",
        "--port",
        "4318",
        "--assets",
        "web",
        "--vault",
        "copy",
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(options.vault, PathBuf::from("copy"));
    assert_eq!(options.assets, PathBuf::from("web"));
    assert_eq!(options.label, "服务器示例库");
    assert_eq!(options.port, 4318);
    assert!(options.access.is_none());
}

#[test]
fn complete_public_group_enables_access() {
    let mut values = vec!["--vault", "copy"];
    for (key, value) in PUBLIC {
        values.extend([key, value]);
    }
    assert!(parse(args(&values)).unwrap().unwrap().access.is_some());
}

#[test]
fn every_partial_public_group_is_rejected() {
    for mask in 1..15 {
        let mut values = vec!["--vault", "copy"];
        for (index, (key, value)) in PUBLIC.iter().enumerate() {
            if mask & (1 << index) != 0 {
                values.extend([*key, *value]);
            }
        }
        assert!(parse(args(&values)).is_err(), "partial group {mask}");
    }
}

#[test]
fn every_value_parameter_rejects_duplicates() {
    for (key, value) in [
        ("--vault", "copy"),
        ("--assets", "web"),
        ("--label", "示例库"),
        ("--port", "4318"),
    ]
    .into_iter()
    .chain(PUBLIC)
    {
        let error = parse(args(&[key, value, key, value])).err().unwrap();
        assert!(error.contains("不得重复"), "{key}: {error}");
    }
}

#[test]
fn missing_and_empty_values_are_rejected() {
    for key in [
        "--vault",
        "--assets",
        "--label",
        "--port",
        "--public-origin",
        "--access-team",
        "--access-audience",
        "--access-email",
    ] {
        for values in [vec![key], vec![key, ""], vec![key, "--vault", "copy"]] {
            let error = parse(args(&values)).err().unwrap();
            assert!(error.contains("缺少值"), "{key}: {error}");
        }
    }
}

#[test]
fn unknown_options_and_positional_arguments_are_rejected() {
    for value in ["--unknown", "copy", "--port=4318"] {
        let error = parse(args(&[value])).err().unwrap();
        assert!(error.contains("未知参数"), "{value}: {error}");
    }
}

#[test]
fn invalid_ports_are_rejected() {
    for port in ["0", "65536", "-1", "abc"] {
        assert!(parse(args(&["--vault", "copy", "--port", port])).is_err());
    }
}

#[test]
fn public_configuration_validation_cannot_fall_back_to_local_mode() {
    for (index, invalid) in [
        "http://studiolo.example.com",
        "https://studiolo.example.com",
        "无效 audience",
        "用户@example.com",
    ]
    .into_iter()
    .enumerate()
    {
        let mut values = vec!["--vault", "copy"];
        for (position, (key, value)) in PUBLIC.iter().enumerate() {
            values.extend([*key, if position == index { invalid } else { *value }]);
        }
        assert!(
            parse(args(&values)).is_err(),
            "invalid public field {index}"
        );
    }
}

#[test]
fn help_needs_no_vault_but_does_not_accept_duplicate_aliases() {
    for help in ["--help", "-h"] {
        assert!(parse(args(&[help])).unwrap().is_none());
        assert!(parse(args(&[help, help])).is_err());
    }
    assert!(parse(args(&["--help", "-h"])).is_err());
    assert!(parse(args(&["--help", "--unknown"])).is_err());
}
