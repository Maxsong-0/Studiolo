//! 私有备份配置参数只在会话模式加 canonical 写者下生效；解析阶段不访问任何文件。
//! 给了却在别的模式下被悄悄忽略比拒绝更坏：生产者以为启用了，其实没有。

use super::parse;
use std::path::Path;

const MESSAGE: &str = "备份配置必须为绝对路径，并配合会话配置与 canonical 写者";

fn arguments(extra: &[&str]) -> Vec<String> {
    [
        "--vault",
        "/fixture/vault",
        "--session-config",
        "/fixture/owner.json",
        "--public-origin",
        "https://studiolo.example.com",
        "--canonical-writer-data",
        "/fixture/writer",
    ]
    .iter()
    .chain(extra.iter())
    .map(|value| (*value).to_owned())
    .collect()
}

fn check(values: Vec<String>) -> Result<Option<super::Options>, String> {
    parse(values.into_iter())
}

fn owned(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn backup_config_lands_in_its_own_field_and_is_absent_by_default() {
    let options = check(arguments(&["--backup-config", "/fixture/backup.json"]))
        .unwrap()
        .unwrap();
    assert_eq!(
        options.backup_config.as_deref(),
        Some(Path::new("/fixture/backup.json"))
    );
    // 默认关闭：不给参数就没有生产者，其余字段一概不受影响。
    assert!(
        check(arguments(&[]))
            .unwrap()
            .unwrap()
            .backup_config
            .is_none()
    );
}

#[test]
fn a_relative_backup_config_path_is_rejected() {
    assert_eq!(
        check(arguments(&["--backup-config", "relative/backup.json"]))
            .err()
            .as_deref(),
        Some(MESSAGE)
    );
}

#[test]
fn a_backup_config_without_a_session_or_a_canonical_writer_is_rejected() {
    let local = owned(&[
        "--vault",
        "/fixture/vault",
        "--backup-config",
        "/fixture/backup.json",
    ]);
    assert_eq!(check(local).err().as_deref(), Some(MESSAGE));
    let without_writer = owned(&[
        "--vault",
        "/fixture/vault",
        "--session-config",
        "/fixture/owner.json",
        "--public-origin",
        "https://studiolo.example.com",
        "--backup-config",
        "/fixture/backup.json",
    ]);
    assert_eq!(check(without_writer).err().as_deref(), Some(MESSAGE));
}

#[test]
fn the_backup_config_may_not_be_given_twice() {
    assert!(
        check(arguments(&[
            "--backup-config",
            "/fixture/backup.json",
            "--backup-config",
            "/fixture/other.json",
        ]))
        .is_err()
    );
}
