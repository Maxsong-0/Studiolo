//! 冲突副本命名的契约：S5 真实样例、`studiolo-canonical` 往返、无扩展名／多点扩展名／
//! 设备名含空格、非 12 位戳、UTC 分钟戳。

use crate::fixture::path;
use studiolo_local_server::sync_plane::naming::{
    CANONICAL_DEVICE, CONFLICTS_DIR, ConflictCopy, ROLLBACK_DIR, copy_path, parse_copy,
    utc_minute_stamp,
};

const STAMP: &str = "202609230332";

#[test]
fn parse_recognizes_headless_sample_from_s5() {
    let sample = path("S5/conflict (Conflicted copy studiolo-169-p2-observer 202609230332).md");
    assert_eq!(
        parse_copy(&sample),
        Some(ConflictCopy {
            original: path("S5/conflict.md"),
            device: "studiolo-169-p2-observer".to_owned(),
            stamp: STAMP.to_owned(),
        })
    );
    assert_eq!(parse_copy(&path("S5/conflict.md")), None);
    assert_eq!(parse_copy(&path("S5/Conflicted copy.md")), None);
}

#[test]
fn canonical_copy_round_trips() {
    let generated = copy_path(
        CONFLICTS_DIR,
        &path("笔记/深/正文.md"),
        CANONICAL_DEVICE,
        STAMP,
    )
    .unwrap();
    assert_eq!(
        generated.as_str(),
        "ops/bridge-conflicts/笔记/深/正文 (Conflicted copy studiolo-canonical 202609230332).md"
    );
    let parsed = parse_copy(&generated).unwrap();
    assert_eq!(
        parsed.original.as_str(),
        "ops/bridge-conflicts/笔记/深/正文.md"
    );
    assert_eq!(
        parsed
            .original
            .as_str()
            .strip_prefix("ops/bridge-conflicts/"),
        Some("笔记/深/正文.md")
    );
    assert_eq!(parsed.device, CANONICAL_DEVICE);
    assert_eq!(parsed.stamp, STAMP);
    let rollback = copy_path(ROLLBACK_DIR, &path("顶层.md"), CANONICAL_DEVICE, STAMP).unwrap();
    assert_eq!(
        rollback.as_str(),
        "ops/bridge-rollback/顶层 (Conflicted copy studiolo-canonical 202609230332).md"
    );
    assert_eq!(
        parse_copy(&rollback).unwrap().original.as_str(),
        "ops/bridge-rollback/顶层.md"
    );
    assert_eq!(
        copy_path("ops/elsewhere", &path("a.md"), CANONICAL_DEVICE, STAMP),
        None
    );
    assert_eq!(
        copy_path("笔记", &path("a.md"), CANONICAL_DEVICE, STAMP),
        None
    );
}

#[test]
fn naming_handles_no_extension_multi_dot_and_spaced_device() {
    let bare = copy_path(CONFLICTS_DIR, &path("附件/手册"), "My Mac", STAMP).unwrap();
    assert_eq!(
        bare.as_str(),
        "ops/bridge-conflicts/附件/手册 (Conflicted copy My Mac 202609230332)"
    );
    let parsed = parse_copy(&bare).unwrap();
    assert_eq!(
        (
            parsed.original.as_str(),
            parsed.device.as_str(),
            parsed.stamp.as_str()
        ),
        ("ops/bridge-conflicts/附件/手册", "My Mac", STAMP)
    );
    let multi = copy_path(CONFLICTS_DIR, &path("包.tar.gz"), CANONICAL_DEVICE, STAMP).unwrap();
    assert_eq!(
        multi.as_str(),
        "ops/bridge-conflicts/包.tar (Conflicted copy studiolo-canonical 202609230332).gz"
    );
    assert_eq!(
        parse_copy(&multi).unwrap().original.as_str(),
        "ops/bridge-conflicts/包.tar.gz"
    );
    let hidden = copy_path(CONFLICTS_DIR, &path(".env"), CANONICAL_DEVICE, STAMP).unwrap();
    assert_eq!(
        hidden.as_str(),
        "ops/bridge-conflicts/.env (Conflicted copy studiolo-canonical 202609230332)"
    );
    assert_eq!(
        parse_copy(&hidden).unwrap().original.as_str(),
        "ops/bridge-conflicts/.env"
    );
    let long = "x".repeat(65);
    for device in ["a(b", "a)b", "a/b", "", "tab\tname", long.as_str()] {
        assert_eq!(
            copy_path(CONFLICTS_DIR, &path("a.md"), device, STAMP),
            None,
            "设备名 {device:?} 必须被拒"
        );
    }
    let nested = parse_copy(&path(
        "a (Conflicted copy d1 202609230332) (Conflicted copy d2 202609230333).md",
    ))
    .unwrap();
    assert_eq!(
        nested.original.as_str(),
        "a (Conflicted copy d1 202609230332).md"
    );
    assert_eq!(
        (nested.device.as_str(), nested.stamp.as_str()),
        ("d2", "202609230333")
    );
    assert_eq!(
        parse_copy(&path("x (Conflicted copy a b 202609230332).md"))
            .unwrap()
            .device,
        "a b"
    );
}

#[test]
fn naming_rejects_non_twelve_digit_stamp() {
    assert_eq!(
        copy_path(CONFLICTS_DIR, &path("a.md"), CANONICAL_DEVICE, "2026092303"),
        None
    );
    assert_eq!(
        copy_path(
            CONFLICTS_DIR,
            &path("a.md"),
            CANONICAL_DEVICE,
            "2026092303321"
        ),
        None
    );
    assert_eq!(
        copy_path(
            CONFLICTS_DIR,
            &path("a.md"),
            CANONICAL_DEVICE,
            "20260923033a"
        ),
        None
    );
    assert_eq!(
        parse_copy(&path("a (Conflicted copy d 2026092303).md")),
        None
    );
    assert_eq!(
        parse_copy(&path("a (Conflicted copy d 2026092303321).md")),
        None
    );
    assert_eq!(
        parse_copy(&path("a (Conflicted copy d 20260923033a).md")),
        None
    );
    assert_eq!(
        parse_copy(&path("(Conflicted copy d 202609230332).md")),
        None
    );
    assert_eq!(
        parse_copy(&path("a (Conflicted copy d/e 202609230332).md")),
        None
    );
    assert_eq!(
        parse_copy(&path("a (Conflicted copy d 202609230332)x.md")),
        None
    );
}

#[test]
fn utc_minute_stamp_formats_utc() {
    assert_eq!(utc_minute_stamp(0).as_deref(), Some("197001010000"));
    assert_eq!(
        utc_minute_stamp(1_790_134_320).as_deref(),
        Some(STAMP),
        "S5 样例的分钟"
    );
    assert_eq!(
        utc_minute_stamp(1_790_134_379).as_deref(),
        Some(STAMP),
        "同一分钟内秒数不影响"
    );
    assert_eq!(
        utc_minute_stamp(1_790_134_380).as_deref(),
        Some("202609230333")
    );
    assert_eq!(utc_minute_stamp(-1), None);
    assert_eq!(utc_minute_stamp(i64::MAX), None);
}
