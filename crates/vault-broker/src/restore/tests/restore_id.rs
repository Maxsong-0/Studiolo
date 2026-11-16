//! lineage 第 2 版带恢复标识（T-212）：写进去、接手只认同一个、格式不对在动手前拒绝、第 1 版照样读得进来。
use super::Fixture;
use crate::restore::{RestoreError, RestoreFault, read_restore_lineage, rebind_new_roots};
use std::fs;

#[test]
fn completed_lineage_carries_the_restore_id() {
    let f = Fixture::restored();
    let receipt = f.rebind().unwrap();
    assert_eq!(receipt.lineage.format_version, 2);
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
}
