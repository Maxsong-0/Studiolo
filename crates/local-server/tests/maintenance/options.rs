use super::super::options::{Command, parse};
use std::ffi::OsString;

fn args(command: &str) -> Vec<OsString> {
    [
        command,
        "--config",
        "/private/config.json",
        "--vault",
        "/vault",
        "--assets",
        "/assets",
        "--writer-data",
        "/writer",
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

#[test]
fn only_closed_commands_accept_fixed_non_secret_roots() {
    for (name, expected) in [
        ("inspect", Command::Inspect),
        ("audit-flush", Command::AuditFlush),
    ] {
        let value = parse(args(name)).unwrap().unwrap();
        assert_eq!(value.command, expected);
        assert_eq!(value.config.to_str(), Some("/private/config.json"));
        assert_eq!(value.vault.to_str(), Some("/vault"));
        assert_eq!(value.assets.to_str(), Some("/assets"));
        assert_eq!(value.writer_data.to_str(), Some("/writer"));
        assert!(value.request.is_none());
    }
    assert!(parse(vec!["--help".into()]).unwrap().is_none());
    for name in [
        "cancel-prepared",
        "preserve-installing",
        "cancel-restore-intent",
    ] {
        let mut input = args(name);
        assert!(parse(input.clone()).is_err());
        input.extend([OsString::from("--request"), "ab".repeat(32).into()]);
        let value = parse(input).unwrap().unwrap();
        assert_eq!(value.command.name(), name);
        assert_eq!(value.request.as_deref(), Some("ab".repeat(32).as_str()));
    }
}

#[test]
fn no_identity_override_automatic_yes_or_unbounded_inputs() {
    for flag in [
        "--yes",
        "--stdin",
        "--issuer",
        "--subject",
        "--seed",
        "--body",
        "--key",
        "--repair",
    ] {
        let mut input = args("inspect");
        input[1] = flag.into();
        assert!(parse(input).is_err());
    }
    for name in [
        "restore",
        "initialize",
        "stop",
        "migrate",
        "clear-freeze",
        "",
    ] {
        assert!(parse(args(name)).is_err());
    }
    for path in ["relative", "/bad\npath", ""] {
        let mut input = args("inspect");
        input[2] = path.into();
        assert!(parse(input).is_err());
    }
    let mut input = args("inspect");
    input[2] = format!("/{}", "x".repeat(4096)).into();
    assert!(parse(input).is_err());
    let mut duplicate = args("inspect");
    duplicate[3] = "--config".into();
    assert!(parse(duplicate).is_err());
}

#[test]
fn request_is_exact_lowercase_digest_only_and_not_accepted_for_inspect() {
    for value in [
        "a".repeat(63),
        "A".repeat(64),
        "g".repeat(64),
        "a".repeat(65),
        "\n".repeat(64),
    ] {
        let mut input = args("cancel-prepared");
        input.extend([OsString::from("--request"), value.into()]);
        assert!(parse(input).is_err());
    }
    for name in ["inspect", "audit-flush"] {
        let mut input = args(name);
        input.extend([OsString::from("--request"), "a".repeat(64).into()]);
        assert!(parse(input).is_err());
    }
}

#[test]
fn maintenance_cancellation_uses_its_own_explicit_selector() {
    let mut input = args("cancel-maintenance");
    assert!(parse(input.clone()).is_err());
    input.extend([OsString::from("--maintenance"), "c".repeat(64).into()]);
    let value = parse(input.clone()).unwrap().unwrap();
    assert!(value.request.is_none());
    assert_eq!(value.maintenance.as_deref(), Some("c".repeat(64).as_str()));
    input[9] = "--request".into();
    assert!(parse(input).is_err());
    let mut input = args("cancel-prepared");
    input.extend([OsString::from("--maintenance"), "c".repeat(64).into()]);
    assert!(parse(input).is_err());
}

#[test]
fn sync_commands_take_the_approval_document_and_purge_takes_a_validated_folder() {
    let mut init = args("sync-init");
    assert!(parse(init.clone()).is_err());
    init.extend([
        OsString::from("--sync-transport"),
        "/private/sync-approval.json".into(),
    ]);
    let value = parse(init.clone()).unwrap().unwrap();
    assert_eq!(value.command, Command::SyncInit);
    assert_eq!(value.command.name(), "sync-init");
    assert_eq!(
        value
            .sync_transport
            .as_deref()
            .and_then(std::path::Path::to_str),
        Some("/private/sync-approval.json")
    );
    assert!(value.folder.is_none() && value.request.is_none());
    let mut with_folder = init.clone();
    with_folder.extend([OsString::from("--folder"), "School".into()]);
    assert!(parse(with_folder).is_err());
    let mut relative = args("sync-init");
    relative.extend([OsString::from("--sync-transport"), "relative.json".into()]);
    assert!(parse(relative).is_err());

    let mut purge = args("sync-purge-residue");
    purge.extend([
        OsString::from("--sync-transport"),
        "/private/sync-approval.json".into(),
    ]);
    assert!(parse(purge.clone()).is_err());
    for bad in [
        "/School", "School/", "a/../b", "", " ", "School\n", "a\\b", "./a",
    ] {
        let mut input = purge.clone();
        input.extend([OsString::from("--folder"), bad.into()]);
        assert!(parse(input).is_err(), "{bad:?}");
    }
    let mut good = purge.clone();
    good.extend([OsString::from("--folder"), "School/子目录".into()]);
    let value = parse(good).unwrap().unwrap();
    assert_eq!(value.command, Command::SyncPurgeResidue);
    assert_eq!(value.folder.as_deref(), Some("School/子目录"));
    let mut duplicate = purge;
    duplicate.extend([
        OsString::from("--sync-transport"),
        "/private/again.json".into(),
    ]);
    assert!(parse(duplicate).is_err());

    // 老命令不接受审批文档参数，新命令也不接受请求指纹。
    let mut inspect = args("inspect");
    inspect.extend([
        OsString::from("--sync-transport"),
        "/private/sync-approval.json".into(),
    ]);
    assert!(parse(inspect).is_err());
    let mut request = args("sync-init");
    request.extend([OsString::from("--request"), "a".repeat(64).into()]);
    assert!(parse(request).is_err());
}

#[cfg(unix)]
#[test]
fn non_utf8_arguments_are_rejected_before_any_io() {
    use std::os::unix::ffi::OsStringExt;
    let mut input = args("inspect");
    input[2] = OsString::from_vec(vec![b'/', 0xff]);
    assert!(parse(input).is_err());
}
