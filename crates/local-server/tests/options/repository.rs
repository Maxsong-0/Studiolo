//! 显式仓库绑定不靠路径猜测，也不绕过既有会话根隔离。

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
fn repository_requires_an_explicit_project_and_absolute_root() {
    let parsed = options(&["--project-repository", "项目/虚构=/tmp/repo"])
        .unwrap()
        .unwrap();
    assert_eq!(
        parsed.project_repository.unwrap(),
        ("项目/虚构".into(), "/tmp/repo".into())
    );
    for value in [
        "/tmp/repo",
        "项目/虚构=relative",
        "other/虚构=/tmp/repo",
        "项目/虚构/子=/tmp/repo",
        "项目/.hidden=/tmp/repo",
        "项目/../虚构=/tmp/repo",
        "项目/虚构=/tmp/secret\nroot",
    ] {
        let error = options(&["--project-repository", value]).err().unwrap();
        assert!(!error.contains(value));
    }
}

#[test]
fn repository_binding_cannot_be_duplicated_or_mixed_with_session_modes() {
    assert!(
        options(&[
            "--project-repository",
            "项目/虚构=/tmp/a",
            "--project-repository",
            "项目/虚构=/tmp/b"
        ])
        .is_err()
    );
    assert!(
        options(&[
            "--project-repository",
            "项目/虚构=/tmp/a",
            "--session-config",
            "/tmp/owner.json",
            "--public-origin",
            "https://studiolo.example.com"
        ])
        .is_err()
    );
    assert!(options(&[]).unwrap().unwrap().project_repository.is_none());
}
