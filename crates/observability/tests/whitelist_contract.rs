//! 白名单诊断的行为／失败／恢复三类契约。
//!
//! 诊断只读：任何一条用例都不会写回配置，也不会"顺手修好"。

use studiolo_core::VaultPath;
use studiolo_observability::health::{
    DiagnosticsError, WhitelistFindingKind, WhitelistSnapshot, diagnose,
};

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("固定测试路径必须合法")
}

fn kinds(snapshot: &WhitelistSnapshot) -> Vec<WhitelistFindingKind> {
    diagnose(snapshot)
        .expect("规模内的快照必须可诊断")
        .findings()
        .iter()
        .map(|finding| finding.kind)
        .collect()
}

#[test]
fn 互不重叠的登记被判定为一致() {
    let snapshot = WhitelistSnapshot {
        sync_whole_folders: vec![path("Shared")],
        generated_regions: vec![path("Generated/index.md")],
        file_operation_writable: vec![path("Notes")],
        human_content_folders: vec![path("Notes")],
        human_content_protected: vec![path("Private")],
        ..WhitelistSnapshot::default()
    };

    let diagnosis = diagnose(&snapshot).expect("规模内的快照必须可诊断");

    assert!(diagnosis.consistent());
    assert_eq!(diagnosis.examined(), 5);
}

#[test]
fn 同一路径既在生成区又在人侧可写范围时报冲突() {
    let snapshot = WhitelistSnapshot {
        generated_regions: vec![path("Notes/index.md")],
        file_operation_writable: vec![path("Notes")],
        ..WhitelistSnapshot::default()
    };

    assert!(kinds(&snapshot).contains(&WhitelistFindingKind::GeneratedRegionIsWritable));
}

#[test]
fn 生成区落在双向搬运范围之内时报冲突() {
    let snapshot = WhitelistSnapshot {
        sync_whole_folders: vec![path("Shared")],
        generated_regions: vec![path("Shared/dashboard.md")],
        ..WhitelistSnapshot::default()
    };

    assert!(kinds(&snapshot).contains(&WhitelistFindingKind::GeneratedRegionInsideSyncScope));
}

#[test]
fn 搬运范围覆盖保护前缀时报冲突() {
    let snapshot = WhitelistSnapshot {
        sync_whole_folders: vec![path("Vault")],
        sync_protected: vec![path("Vault/Private")],
        ..WhitelistSnapshot::default()
    };

    assert!(kinds(&snapshot).contains(&WhitelistFindingKind::SyncScopeCoversProtected));
}

#[test]
fn 人侧可写范围覆盖保护前缀时报冲突() {
    let snapshot = WhitelistSnapshot {
        file_operation_writable: vec![path("Vault")],
        human_content_protected: vec![path("Vault/Private")],
        ..WhitelistSnapshot::default()
    };

    assert!(kinds(&snapshot).contains(&WhitelistFindingKind::WritableCoversProtected));
}

#[test]
fn 人写目录整体落在保护前缀之内时报冲突() {
    let snapshot = WhitelistSnapshot {
        human_content_folders: vec![path("Private/Notes")],
        human_content_protected: vec![path("Private")],
        ..WhitelistSnapshot::default()
    };

    assert!(kinds(&snapshot).contains(&WhitelistFindingKind::HumanFolderInsideProtected));
}

#[test]
fn 同一张表内重复登记同一路径时报冲突() {
    let snapshot = WhitelistSnapshot {
        sync_whole_files: vec![path("Shared/a.md"), path("Shared/a.md")],
        ..WhitelistSnapshot::default()
    };

    assert!(kinds(&snapshot).contains(&WhitelistFindingKind::DuplicateRegistration));
}

#[test]
fn 整文件登记被同范围整目录规则遮蔽时报冲突() {
    let snapshot = WhitelistSnapshot {
        sync_whole_files: vec![path("Shared/a.md")],
        sync_whole_folders: vec![path("Shared")],
        ..WhitelistSnapshot::default()
    };

    assert!(kinds(&snapshot).contains(&WhitelistFindingKind::WholeFileShadowedByFolder));
}

#[test]
fn 前缀只在路径分段边界上算覆盖因而同名前缀不会误报() {
    let snapshot = WhitelistSnapshot {
        sync_whole_folders: vec![path("Share")],
        sync_protected: vec![path("Shared")],
        ..WhitelistSnapshot::default()
    };

    assert!(
        diagnose(&snapshot)
            .expect("规模内的快照必须可诊断")
            .consistent()
    );
}

#[test]
fn 登记总数超过诊断预算时失败关闭而不是报告一致() {
    let mut snapshot = WhitelistSnapshot::default();
    for index in 0..4_097 {
        snapshot
            .sync_whole_files
            .push(path(&format!("Shared/file-{index}.md")));
    }

    assert_eq!(diagnose(&snapshot), Err(DiagnosticsError::TooManyEntries));
}

#[test]
fn 把冲突登记从可写范围移走之后诊断重新报告一致() {
    let broken = WhitelistSnapshot {
        generated_regions: vec![path("Notes/index.md")],
        file_operation_writable: vec![path("Notes")],
        ..WhitelistSnapshot::default()
    };
    assert!(
        !diagnose(&broken)
            .expect("规模内的快照必须可诊断")
            .consistent()
    );

    let repaired = WhitelistSnapshot {
        generated_regions: vec![path("Generated/index.md")],
        file_operation_writable: vec![path("Notes")],
        ..WhitelistSnapshot::default()
    };

    assert!(
        diagnose(&repaired)
            .expect("规模内的快照必须可诊断")
            .consistent()
    );
}

#[test]
fn 诊断不修改输入快照() {
    let snapshot = WhitelistSnapshot {
        generated_regions: vec![path("Notes/index.md")],
        file_operation_writable: vec![path("Notes")],
        ..WhitelistSnapshot::default()
    };
    let before = snapshot.clone();

    let _ = diagnose(&snapshot).expect("规模内的快照必须可诊断");

    assert_eq!(snapshot, before);
}
