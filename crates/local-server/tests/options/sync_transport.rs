//! 双向 Bridge 审批文档参数只在会话模式加 canonical 写者下生效；解析阶段不访问任何文件。

use super::parse;
use std::path::Path;

const MESSAGE: &str = "双向 Bridge 审批文档必须为绝对路径，并配合会话配置与 canonical 写者";

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

#[test]
fn approval_path_lands_in_its_own_field_and_is_absent_by_default() {
    let options = check(arguments(&[
        "--sync-transport",
        "/fixture/sync-approval.json",
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(
        options.sync_transport.as_deref(),
        Some(Path::new("/fixture/sync-approval.json"))
    );
    assert!(options.writer_data.is_some() && options.session.is_some());
    // 默认关闭：不给参数就没有平面，其余字段一概不受影响。
    assert!(
        check(arguments(&[]))
            .unwrap()
            .unwrap()
            .sync_transport
            .is_none()
    );
}

#[test]
fn relative_path_is_rejected_verbatim() {
    assert_eq!(
        check(arguments(&["--sync-transport", "relative/approval.json"]))
            .err()
            .as_deref(),
        Some(MESSAGE)
    );
}

#[test]
fn approval_without_a_session_config_is_rejected() {
    let local = [
        "--vault",
        "/fixture/vault",
        "--sync-transport",
        "/fixture/sync-approval.json",
    ]
    .iter()
    .map(|value| (*value).to_owned())
    .collect();
    assert_eq!(check(local).err().as_deref(), Some(MESSAGE));
}

#[test]
fn approval_without_a_canonical_writer_is_rejected() {
    let session = [
        "--vault",
        "/fixture/vault",
        "--session-config",
        "/fixture/owner.json",
        "--public-origin",
        "https://studiolo.example.com",
        "--sync-transport",
        "/fixture/sync-approval.json",
    ]
    .iter()
    .map(|value| (*value).to_owned())
    .collect();
    assert_eq!(check(session).err().as_deref(), Some(MESSAGE));
}
