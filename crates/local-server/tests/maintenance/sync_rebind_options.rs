//! 新命令只接固定非秘密根参数，不增加自动确认或迁移开关。

use super::super::options::{Command, parse};
use std::ffi::OsString;

fn args() -> Vec<OsString> {
    [
        "sync-rebind",
        "--config",
        "/private/config.json",
        "--vault",
        "/vault",
        "--assets",
        "/assets",
        "--writer-data",
        "/writer",
        "--sync-transport",
        "/private/new-approval.json",
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

#[test]
fn rebind_accepts_only_its_complete_approval_and_common_roots() {
    let options = parse(args()).unwrap().unwrap();
    assert_eq!(options.command, Command::SyncRebind);
    assert_eq!(options.command.name(), "sync-rebind");
    assert!(options.folder.is_none() && options.request.is_none());
    assert_eq!(
        options.sync_transport.unwrap().to_str(),
        Some("/private/new-approval.json")
    );
    let mut incomplete = args();
    incomplete.truncate(9);
    assert!(parse(incomplete).is_err());
    for flag in [
        "--request",
        "--folder",
        "--yes",
        "--migrate",
        "--old-config",
    ] {
        let mut bad = args();
        bad[9] = flag.into();
        assert!(parse(bad).is_err());
    }
    for path in ["relative.json", "/bad\nname", ""] {
        let mut bad = args();
        bad[10] = path.into();
        assert!(parse(bad).is_err());
    }
    let mut duplicate = args();
    duplicate.extend(["--sync-transport".into(), "/duplicate.json".into()]);
    assert!(parse(duplicate).is_err());
}
