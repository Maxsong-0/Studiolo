//! 封闭参数解析契约。
use super::*;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn serve_requires_absolute_paths_and_at_least_one_allow() {
    let parsed = parse(&strings(&[
        "serve",
        "--vault",
        "/v",
        "--allow",
        "项目/Studiolo",
        "--allow",
        "_规则",
        "--data",
        "/d",
    ]))
    .unwrap();
    assert_eq!(
        parsed,
        Command::Serve(ServeOptions {
            index_cache: None,
            vault: Some(PathBuf::from("/v")),
            replica_root: None,
            replica_max_age_secs: 300,
            allow: vec!["项目/Studiolo".into(), "_规则".into()],
            data: PathBuf::from("/d"),
            port: 4400,
        })
    );
    assert!(
        parse(&strings(&[
            "serve", "--vault", "v", "--allow", "a", "--data", "/d"
        ]))
        .is_err()
    );
    assert!(parse(&strings(&["serve", "--vault", "/v", "--data", "/d"])).is_err());
    assert!(
        parse(&strings(&[
            "serve", "--vault", "/v", "--allow", "a", "--data", "/d", "--port", "80"
        ]))
        .is_err()
    );
    assert!(
        parse(&strings(&[
            "serve", "--vault", "/v", "--allow", "a", "--data", "/d", "--extra", "x"
        ]))
        .is_err()
    );
}

#[test]
fn admin_subcommands_are_closed() {
    assert_eq!(
        parse(&strings(&[
            "admin",
            "type-issue",
            "--data",
            "/d",
            "--type",
            "claude"
        ]))
        .unwrap(),
        Command::Admin(AdminCommand::TypeIssue {
            data: PathBuf::from("/d"),
            agent_type: "claude".into()
        })
    );
    assert!(parse(&strings(&["admin", "type-issue", "--data", "/d"])).is_err());
    assert!(parse(&strings(&["admin", "init", "--data", "/d", "--type", "x"])).is_err());
    assert!(parse(&strings(&["admin", "nuke", "--data", "/d"])).is_err());
    assert!(matches!(parse(&strings(&[])).unwrap(), Command::Help));
    assert!(matches!(
        parse(&strings(&[
            "admin", "reads", "--data", "/d", "--limit", "5"
        ]))
        .unwrap(),
        Command::Admin(AdminCommand::Reads { limit: 5, .. })
    ));
}

#[test]
fn index_cache_is_an_optional_single_absolute_path() {
    let base = ["serve", "--vault", "/v", "--allow", "a", "--data", "/d"];
    let mut args = strings(&base);
    args.extend(strings(&["--index-cache", "/cache"]));
    let Command::Serve(options) = parse(&args).unwrap() else {
        panic!("serve")
    };
    assert_eq!(options.index_cache, Some(PathBuf::from("/cache")));
    for suffix in [
        vec!["--index-cache", "relative"],
        vec!["--index-cache"],
        vec!["--index-cache", "/one", "--index-cache", "/two"],
    ] {
        let mut args = strings(&base);
        args.extend(strings(&suffix));
        assert!(parse(&args).is_err());
    }
}
