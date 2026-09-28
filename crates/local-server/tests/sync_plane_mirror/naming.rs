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

#[test]
fn conflict_copy_count_recognizes_both_device_names_once_per_path() {
    use std::collections::BTreeMap;
    use studiolo_local_server::sync_plane::driver::observe::conflict_copy_count;
    use studiolo_sync_bridge::{Binding, FileVersion, Inventory, Presence, Side};

    let binding = Binding {
        link_id: [1; 16],
        canonical_vault: [2; 16],
        remote_vault: [3; 16],
        canonical_root_token: [4; 32],
        mirror_root_token: [5; 32],
        scope_hash: [6; 32],
        scope_epoch: 1,
    };
    let present = || {
        Presence::Present(FileVersion {
            revision: [7; 32],
            sha256: [7; 32],
            bytes: 1,
        })
    };
    let inventory = |side: Side, entries: BTreeMap<_, _>| Inventory {
        binding: binding.clone(),
        side,
        sequence: 1,
        complete: true,
        entries,
    };
    let headless_copy = path("n (Conflicted copy iPhone 202609230332).md");
    let canonical_copy = path("n (Conflicted copy studiolo-canonical 202609230332).md");
    let canonical = inventory(
        Side::Canonical,
        BTreeMap::from([
            (path("n.md"), present()),
            (headless_copy.clone(), present()),
            (
                path("x (Conflicted copy d 202609230332).md"),
                Presence::Missing,
            ),
        ]),
    );
    let mirror = inventory(
        Side::Mirror,
        BTreeMap::from([
            (headless_copy, present()),
            (canonical_copy, present()),
            (
                path("x (Conflicted copy d 202609230332).md"),
                Presence::Missing,
            ),
        ]),
    );
    // headless 副本两侧都有只数一次；studiolo-canonical 副本只在镜像侧；显式缺失的不数；原文件不数。
    assert_eq!(conflict_copy_count(&canonical, &mirror), 2);
    assert_eq!(conflict_copy_count(&mirror, &canonical), 2);
}

#[test]
fn copy_names_longer_than_name_max_are_shortened_with_a_digest() {
    // 69 个汉字的标题本身合法（207 字节），加上副本标记后末段超过 255 字节（评审 P1-3）。
    let title = "标".repeat(69);
    let original = path(&format!("项目/{title}.md"));
    let copy = copy_path(CONFLICTS_DIR, &original, CANONICAL_DEVICE, STAMP).expect("截短后放得下");
    let last = copy.as_str().rsplit('/').next().unwrap();
    assert!(last.len() <= 255, "{}", last.len());
    assert!(last.ends_with(&format!(" (Conflicted copy {CANONICAL_DEVICE} {STAMP}).md")));
    assert!(last.contains('~'));
    // 同名前缀的两个长名不会撞成同一个副本名；同一原名每次结果相同。
    let other = path(&format!("项目/{title}乙.md"));
    let other_copy = copy_path(CONFLICTS_DIR, &other, CANONICAL_DEVICE, STAMP).unwrap();
    assert_ne!(copy, other_copy);
    assert_eq!(
        copy_path(CONFLICTS_DIR, &original, CANONICAL_DEVICE, STAMP),
        Some(copy)
    );
    // 短名照旧不截。
    let short = copy_path(CONFLICTS_DIR, &path("项目/短.md"), CANONICAL_DEVICE, STAMP).unwrap();
    assert!(!short.as_str().contains('~'));
}
