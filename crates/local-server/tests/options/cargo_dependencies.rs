//! 清单正文读取必须显式开启，不能由目录绑定或HTTP参数隐式取得。

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
fn cargo_reading_is_disabled_by_default_and_requires_a_binding() {
    assert!(!options(&[]).unwrap().unwrap().cargo_dependencies);
    let binding = ["--project-repository", "项目/虚构=/tmp/repo"];
    assert!(!options(&binding).unwrap().unwrap().cargo_dependencies);
    assert!(options(&["--cargo-dependencies"]).is_err());
    for arguments in [
        vec!["--cargo-dependencies", binding[0], binding[1]],
        vec![binding[0], binding[1], "--cargo-dependencies"],
    ] {
        assert!(options(&arguments).unwrap().unwrap().cargo_dependencies);
    }
}

#[test]
fn cargo_flag_rejects_values_duplicates_and_session_modes() {
    for extra in [
        vec!["--cargo-dependencies", "private-value"],
        vec!["--cargo-dependencies", "--cargo-dependencies"],
        vec![
            "--cargo-dependencies",
            "--session-config",
            "/tmp/owner.json",
            "--public-origin",
            "https://studiolo.example.com",
        ],
        vec!["--cargo-dependencies", "--canonical"],
    ] {
        let args: Vec<_> = ["--project-repository", "项目/虚构=/tmp/repo"]
            .into_iter()
            .chain(extra)
            .collect();
        let error = options(&args).err().expect("无效能力组合应拒绝");
        assert!(!error.contains("private-value"));
        assert!(!error.contains("/tmp/owner.json"));
    }
}

#[test]
fn help_describes_the_flag_without_requiring_io() {
    assert!(
        options(&["--help", "--cargo-dependencies"])
            .unwrap()
            .is_none()
    );
}
