//! 镜像完整观察的契约：中文路径与空格、链接与 FIFO、显式 Missing、排除前缀、字节上限、
//! 硬排除名字与 `SyncScope` 同一口径。

use crate::fixture::{ScopeFixture, binding, path, private_dir, selection, sha256, view, write};
use std::{fs, os::unix::fs::symlink, path::Path};
use studiolo_host_config::SyncDirectoryDecision;
use studiolo_local_server::sync_plane::mirror::observe;
use studiolo_sync_bridge::{Presence, Side};

fn fifo(target: &Path) {
    rustix::fs::mknodat(
        rustix::fs::CWD,
        target,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )
    .unwrap();
}

#[test]
fn observe_hashes_unicode_and_space_paths() {
    let fixture = ScopeFixture::new();
    let scope = fixture.scope();
    let selection = selection(&scope);
    let mirror = private_dir();
    write(
        mirror.path(),
        "笔记/深 层/中文 文件.md",
        "第一行\n".as_bytes(),
    );
    write(mirror.path(), "笔记/a.md", b"a");
    write(mirror.path(), "附件/手册", b"%PDF-1.7");
    let observed = observe(&view(mirror.path()), &selection, &binding(), 5, &[]).unwrap();
    assert!(observed.inventory.complete);
    assert_eq!(observed.inventory.side, Side::Mirror);
    assert_eq!(observed.inventory.sequence, 5);
    assert_eq!(observed.inventory.binding, binding());
    let entries = &observed.inventory.entries;
    assert_eq!(entries.len(), 3);
    match entries.get(&path("笔记/深 层/中文 文件.md")) {
        Some(Presence::Present(present)) => {
            assert_eq!(present.sha256, sha256("第一行\n".as_bytes()));
            assert_eq!(present.revision, present.sha256, "镜像没有 Broker revision");
            assert_eq!(present.bytes, 10);
        }
        other => panic!("中文路径应是 Present：{other:?}"),
    }
    assert!(matches!(
        entries.get(&path("附件/手册")),
        Some(Presence::Present(_))
    ));
    assert_eq!(observed.usage.objects, 3);
    assert_eq!(observed.usage.bytes, 10 + 1 + 8);
}

#[test]
fn observe_marks_links_and_fifos_incomplete() {
    let fixture = ScopeFixture::new();
    let scope = fixture.scope();
    let selection = selection(&scope);
    let linked = private_dir();
    write(linked.path(), "笔记/真.md", b"real");
    symlink(
        linked.path().join("笔记/真.md"),
        linked.path().join("笔记/链.md"),
    )
    .unwrap();
    let observed = observe(&view(linked.path()), &selection, &binding(), 1, &[]).unwrap();
    assert!(!observed.inventory.complete, "符号链接使观察不完整");
    assert!(matches!(
        observed.inventory.entries.get(&path("笔记/真.md")),
        Some(Presence::Present(_))
    ));
    assert!(!observed.inventory.entries.contains_key(&path("笔记/链.md")));
    let piped = private_dir();
    write(piped.path(), "笔记/真.md", b"real");
    fifo(&piped.path().join("笔记/管.md"));
    let observed = observe(&view(piped.path()), &selection, &binding(), 2, &[]).unwrap();
    assert!(!observed.inventory.complete, "FIFO 使观察不完整且不阻塞");
    assert_eq!(observed.usage.objects, 1);
    let drifted = private_dir();
    let stale = view(drifted.path());
    let replaced = private_dir();
    let moved = studiolo_local_server::sync_plane::mirror::MirrorView::new(
        replaced.path().to_path_buf(),
        stale.identity(),
    );
    assert_eq!(
        observe(&moved, &selection, &binding(), 3, &[]).err(),
        Some(studiolo_local_server::sync_plane::PlaneError::Observation),
        "视图身份漂移整轮拒绝"
    );
}

#[test]
fn observe_lists_required_missing_explicitly() {
    let fixture = ScopeFixture::new();
    let scope = fixture.scope();
    let selection = selection(&scope);
    let mirror = private_dir();
    let required = [path("笔记/缺.md"), path("笔记/深/更缺.md")];
    let observed = observe(&view(mirror.path()), &selection, &binding(), 1, &required).unwrap();
    assert!(observed.inventory.complete, "目录锚点尚不存在也是完整观察");
    let entries = &observed.inventory.entries;
    assert_eq!(entries.get(&path("笔记/缺.md")), Some(&Presence::Missing));
    assert_eq!(
        entries.get(&path("笔记/深/更缺.md")),
        Some(&Presence::Missing)
    );
    assert_eq!(
        entries.get(&path("附件/手册")),
        Some(&Presence::Missing),
        "精确文件规则缺失也显式列出"
    );
    assert_eq!(entries.len(), 3);
    assert_eq!(observed.usage.objects, 0);
    let outside = [path("其它/x.md")];
    let outside = observe(&view(mirror.path()), &selection, &binding(), 2, &outside).unwrap();
    assert!(
        !outside.inventory.complete,
        "范围外的 required 无法证明缺失"
    );
}

#[test]
fn observe_never_descends_excluded_prefixes() {
    let fixture = ScopeFixture::new();
    let scope = fixture.scope();
    let selection = selection(&scope);
    let mirror = private_dir();
    write(mirror.path(), "笔记/自动汇总.md", b"generated");
    write(mirror.path(), "笔记/a.md", b"a");
    let observed = observe(&view(mirror.path()), &selection, &binding(), 1, &[]).unwrap();
    assert!(observed.inventory.complete);
    assert!(
        !observed
            .inventory
            .entries
            .contains_key(&path("笔记/自动汇总.md"))
    );
    assert!(observed.inventory.entries.contains_key(&path("笔记/a.md")));
    assert_eq!(observed.usage.objects, 1);
    let demanded = observe(
        &view(mirror.path()),
        &selection,
        &binding(),
        2,
        &[path("笔记/自动汇总.md")],
    )
    .unwrap();
    assert!(
        !demanded.inventory.complete,
        "排除前缀不能出现在 required 里"
    );
}

#[test]
fn observe_rejects_oversized_files() {
    let fixture = ScopeFixture::new();
    let scope = fixture.scope();
    let selection = selection(&scope);
    let mirror = private_dir();
    write(mirror.path(), "笔记/大.bin", &vec![7_u8; 65_537]);
    write(mirror.path(), "笔记/小.md", b"small");
    let observed = observe(&view(mirror.path()), &selection, &binding(), 1, &[]).unwrap();
    assert!(!observed.inventory.complete, "超过规则 max_bytes 即拒绝");
    assert!(
        !observed
            .inventory
            .entries
            .contains_key(&path("笔记/大.bin"))
    );
    assert!(observed.inventory.entries.contains_key(&path("笔记/小.md")));
    assert_eq!(observed.usage.objects, 1);
    fs::write(mirror.path().join("笔记/大.bin"), vec![7_u8; 65_536]).unwrap();
    let limit = observe(&view(mirror.path()), &selection, &binding(), 2, &[]).unwrap();
    assert!(limit.inventory.complete, "恰好等于上限可以");
    assert_eq!(limit.usage.bytes, 65_536 + 5);
}

#[test]
fn observe_skips_hard_excluded_names_like_scope() {
    let fixture = ScopeFixture::new();
    let scope = fixture.scope();
    let selection = selection(&scope);
    let mirror = private_dir();
    let files = [
        "笔记/ops/x.md",
        "笔记/raw/y.md",
        "笔记/.hidden/z.md",
        "笔记/TASKS.md",
        "笔记/skills/s.md",
        "笔记/Sources/t.md",
    ];
    for file in files {
        write(mirror.path(), file, b"excluded");
    }
    write(
        mirror.path(),
        "笔记/sub/tasks.md/inner.md",
        b"under reserved dir",
    );
    write(mirror.path(), "笔记/正常.md", b"kept");
    let observed = observe(&view(mirror.path()), &selection, &binding(), 1, &[]).unwrap();
    assert!(observed.inventory.complete, "硬排除只是不计入，不算拒绝项");
    let kept: Vec<&str> = observed
        .inventory
        .entries
        .keys()
        .map(|key| key.as_str())
        .collect();
    assert_eq!(kept, vec!["笔记/正常.md", "附件/手册"]);
    assert_eq!(observed.usage.objects, 1);
    for file in files {
        assert_eq!(
            scope.policy_for_observation(scope.binding(), &path(file)),
            Ok(None),
            "{file} 在 SyncScope 里同样不可读正文"
        );
    }
    for directory in [
        "笔记/ops",
        "笔记/raw",
        "笔记/.hidden",
        "笔记/skills",
        "笔记/Sources",
        "笔记/sub/tasks.md",
    ] {
        assert_eq!(
            scope.directory_for_observation(scope.binding(), &path(directory)),
            Ok(SyncDirectoryDecision::Skip),
            "{directory} 在 SyncScope 里同样不下探"
        );
    }
    assert_eq!(
        scope.directory_for_observation(scope.binding(), &path("笔记/sub")),
        Ok(SyncDirectoryDecision::Traverse)
    );
}
