//! lineage 带恢复标识（第 2 版）和之前用过的恢复标识（第 3 版，T-212）：写进去、接手只认同一个、格式不对在
//! 动手前拒绝、在恢复出来的根的封存上再恢复时不许复用、第 1、2 版照样读得进来。
use super::{Fixture, copy_tree};
use crate::restore::{
    NewRootRebind, RestoreError, RestoreFault, read_restore_lineage, rebind_new_roots, run,
};
use std::{fs, path::Path};

#[test]
fn completed_lineage_carries_the_restore_id() {
    let f = Fixture::restored();
    let receipt = f.rebind().unwrap();
    assert_eq!(receipt.lineage.format_version, 3);
    assert!(receipt.lineage.earlier_restores.is_empty());
    assert_eq!(receipt.lineage.restore_id.as_deref(), Some("restore-0001"));
    let read = read_restore_lineage(&f.new_data).unwrap().unwrap();
    assert_eq!(read.restore_id.as_deref(), Some("restore-0001"));
}

#[test]
fn resuming_under_another_restore_id_is_refused() {
    let f = Fixture::restored();
    assert!(matches!(
        f.rebind_with_fault(RestoreFault::AfterLineage),
        Err(RestoreError::InterruptedByFault)
    ));
    let mut other = f.request();
    other.resume_interrupted = true;
    other.restore_id = "restore-0002";
    assert!(matches!(
        rebind_new_roots(&other),
        Err(RestoreError::RestoreIdMismatch)
    ));
    let mut same = f.request();
    same.resume_interrupted = true;
    let receipt = rebind_new_roots(&same).unwrap();
    assert!(receipt.lineage.is_completed());
    assert_eq!(receipt.lineage.restore_id.as_deref(), Some("restore-0001"));
}

#[test]
fn malformed_restore_id_is_refused_before_anything_is_written() {
    let f = Fixture::restored();
    let long = "x".repeat(56);
    for bad in ["", "has space", long.as_str(), "slash/no", "中文"] {
        let mut request = f.request();
        request.restore_id = bad;
        assert!(
            matches!(
                rebind_new_roots(&request),
                Err(RestoreError::InvalidRestoreId)
            ),
            "{bad}"
        );
    }
    assert!(read_restore_lineage(&f.new_data).unwrap().is_none());
}

#[test]
fn first_version_lineage_still_reads_and_mixed_versions_are_refused() {
    let f = Fixture::restored();
    f.rebind().unwrap();
    let path = f.new_data.join("restore-v1/lineage.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["format_version"] = 1.into();
    value.as_object_mut().unwrap().remove("restore_id");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let read = read_restore_lineage(&f.new_data).unwrap().unwrap();
    assert_eq!(read.format_version, 1);
    assert_eq!(read.restore_id, None);
    // 第 2 版缺恢复标识、第 1 版带恢复标识：都按未知版本拒绝。
    value["format_version"] = 2.into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        read_restore_lineage(&f.new_data),
        Err(RestoreError::UnsupportedLineage)
    ));
    value["format_version"] = 1.into();
    value["restore_id"] = "restore-0001".into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        read_restore_lineage(&f.new_data),
        Err(RestoreError::UnsupportedLineage)
    ));
    // 第 2 版（有恢复标识、没有之前那一串）照样读得进来；第 2 版带着那一串按未知版本拒绝。
    value["format_version"] = 2.into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let read = read_restore_lineage(&f.new_data).unwrap().unwrap();
    assert_eq!(read.restore_id.as_deref(), Some("restore-0001"));
    value["earlier_restores"] = serde_json::json!(["restore-0000"]);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        read_restore_lineage(&f.new_data),
        Err(RestoreError::UnsupportedLineage)
    ));
    // 第 3 版那一串要自洽：含这次的、重复、不合法的标识都拒绝。
    value["format_version"] = 3.into();
    for bad in [
        serde_json::json!(["restore-0001"]),
        serde_json::json!(["restore-0000", "restore-0000"]),
        serde_json::json!(["has space"]),
    ] {
        value["earlier_restores"] = bad.clone();
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            matches!(
                read_restore_lineage(&f.new_data),
                Err(RestoreError::UnsupportedLineage)
            ),
            "{bad}"
        );
    }
    value["earlier_restores"] = serde_json::json!(["restore-0000"]);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let read = read_restore_lineage(&f.new_data).unwrap().unwrap();
    assert_eq!(read.earlier_restores, ["restore-0000"]);
}

/// 封存的那个根本身是恢复出来的（data 里带着它已完成的 lineage）：再恢复时新 lineage 记下之前用过的全部
/// 恢复标识；这次的标识是其中任何一个都在动手前拒绝（Auth 里留着那次的撤权记录，启动闸门会认旧记录，
/// Codex 第二意见第 1 条），一个字节都不写。
#[test]
fn restoring_a_restored_root_records_every_earlier_restore_and_refuses_reuse() {
    let f = Fixture::restored();
    f.rebind().unwrap();
    let base = f.new_data.parent().unwrap().parent().unwrap();
    let second = copy_pair(base, &f.new_vault, &f.new_data, "second");
    let refused = |pair: &(std::path::PathBuf, std::path::PathBuf), id: &str| {
        let lineage_before = fs::read(pair.1.join("restore-v1/lineage.json")).unwrap();
        let request = request_for(&f, pair, id, "cp-2026-09-18-0002");
        assert!(
            matches!(
                rebind_new_roots(&request),
                Err(RestoreError::RestoreIdReused)
            ),
            "{id}"
        );
        assert_eq!(
            fs::read(pair.1.join("restore-v1/lineage.json")).unwrap(),
            lineage_before,
            "拒绝时 lineage 不能被改"
        );
    };
    refused(&second, "restore-0001");
    let receipt = rebind_new_roots(&request_for(
        &f,
        &second,
        "restore-0002",
        "cp-2026-09-18-0002",
    ))
    .unwrap();
    assert_eq!(receipt.lineage.earlier_restores, ["restore-0001"]);
    assert_eq!(receipt.lineage.restore_id.as_deref(), Some("restore-0002"));

    // 第三代：前两次的标识都不许再用，新的那一串按先后记全。
    let third = copy_pair(base, &second.0, &second.1, "third");
    for reused in ["restore-0001", "restore-0002"] {
        refused(&third, reused);
    }
    let receipt = rebind_new_roots(&request_for(
        &f,
        &third,
        "restore-0003",
        "cp-2026-09-19-0003",
    ))
    .unwrap();
    assert_eq!(
        receipt.lineage.earlier_restores,
        ["restore-0001", "restore-0002"]
    );
    assert!(receipt.lineage.is_completed());
}

/// 之前用过的那一串有上限（256 个）：已经有 255 个之前的（加上它自己的正好 256 个；标识都按最长的 55 个字符算，
/// 整份 lineage 照样在单条记录的大小上限之内）还能再恢复一次；已经有 256 个的再恢复就拒绝，lineage 一个字节都
/// 不改。停在重绑定中途的同一次恢复，接手时沿用第一次算好的那一串，不把自己加进去（Codex 复看修复时指出没有用例）。
#[test]
fn the_earlier_restore_chain_is_capped_and_an_interrupted_rebind_keeps_it() {
    let f = Fixture::restored();
    f.rebind().unwrap();
    let path = f.new_data.join("restore-v1/lineage.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let longest: Vec<String> = (0..255).map(|n| format!("{n:0>55}")).collect();
    value["earlier_restores"] = serde_json::json!(longest);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let base = f.new_data.parent().unwrap().parent().unwrap();

    let interrupted = copy_pair(base, &f.new_vault, &f.new_data, "interrupted");
    let checkpoint = "cp-2026-09-18-0002";
    assert!(matches!(
        run(
            &request_for(&f, &interrupted, "restore-0002", checkpoint),
            RestoreFault::AfterLineage
        ),
        Err(RestoreError::InterruptedByFault)
    ));
    let mut resume = request_for(&f, &interrupted, "restore-0002", checkpoint);
    resume.resume_interrupted = true;
    let receipt = rebind_new_roots(&resume).unwrap();
    assert_eq!(receipt.lineage.earlier_restores.len(), 256);
    assert_eq!(
        receipt.lineage.earlier_restores.last().map(String::as_str),
        Some("restore-0001")
    );
    assert_eq!(
        read_restore_lineage(&interrupted.1).unwrap().unwrap(),
        receipt.lineage
    );

    let over = copy_pair(base, &interrupted.0, &interrupted.1, "over");
    let before = fs::read(over.1.join("restore-v1/lineage.json")).unwrap();
    assert!(matches!(
        rebind_new_roots(&request_for(
            &f,
            &over,
            "restore-0003",
            "cp-2026-09-19-0003"
        )),
        Err(RestoreError::UnsupportedLineage)
    ));
    assert_eq!(
        fs::read(over.1.join("restore-v1/lineage.json")).unwrap(),
        before
    );
}

/// 把一对已经可服务的目录整树复制到 `base/<name>/`，模拟「封存 → 取回 → 放到又一个新目录」。
fn copy_pair(
    base: &Path,
    vault: &Path,
    data: &Path,
    name: &str,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let pair = (base.join(name).join("vault"), base.join(name).join("data"));
    copy_tree(vault, &pair.0);
    copy_tree(data, &pair.1);
    pair
}

fn request_for<'a>(
    f: &Fixture,
    pair: &'a (std::path::PathBuf, std::path::PathBuf),
    restore_id: &'a str,
    checkpoint: &str,
) -> NewRootRebind<'a> {
    let template = f.request();
    NewRootRebind {
        new_vault_root: &pair.0,
        new_data_root: &pair.1,
        expected_vault_uuid: template.expected_vault_uuid,
        expected_pair_id: template.expected_pair_id,
        checkpoint_id: checkpoint.to_owned(),
        restore_id,
        parent_lineage: None,
        resume_interrupted: false,
    }
}
