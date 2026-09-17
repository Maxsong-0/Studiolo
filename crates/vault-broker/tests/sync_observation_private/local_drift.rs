//! 实际文件系统与受控既有私有 hook；不把重新读取后的成功冒充原调查仍有效。
use super::*;
use crate::ReadOnlyVault;
use std::os::unix::fs::symlink;
use std::sync::atomic::Ordering;

#[test]
fn local_metadata_to_finish_new_remove_replace_or_directory_swap_rejects_whole_batch() {
    for attack in 0..5 {
        let f = Fixture::new();
        f.write("notes/a", b"before");
        let reader = ReadOnlyVault::open(&f.root).unwrap();
        let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
        match attack {
            0 => f.write("notes/new", b"new"),
            1 => fs::remove_file(f.root.join("notes/a")).unwrap(),
            2 => {
                f.write("notes/tmp", b"before");
                fs::rename(f.root.join("notes/tmp"), f.root.join("notes/a")).unwrap();
            }
            3 => {
                fs::rename(f.root.join("notes"), f.root.join("previous")).unwrap();
                f.write("notes/a", b"before");
            }
            _ => {
                fs::remove_file(f.root.join("notes/a")).unwrap();
                symlink("/etc/passwd", f.root.join("notes/a")).unwrap();
            }
        }
        reset();
        assert!(
            reader
                .finish_scoped_observation(survey, &[path("notes/new")])
                .is_err()
        );
        assert_eq!(count(1), 0);
    }
}

#[test]
fn local_body_phase_and_final_drift_never_return_partial_facts() {
    for (phase, growth) in [(1, false), (2, false), (3, false), (3, true)] {
        let f = Fixture::new();
        f.write("notes/a", &vec![b'a'; 65536]);
        let reader = ReadOnlyVault::open(&f.root).unwrap();
        let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
        let target = f.root.join("notes/a");
        let _fault = fault(phase, move || {
            fs::OpenOptions::new()
                .write(true)
                .open(target)
                .unwrap()
                .set_len(if growth { 256 * 1024 * 1024 } else { 17 })
                .unwrap();
        });
        reset();
        assert!(reader.finish_scoped_observation(survey, &[]).is_err());
        assert!(count(phase) > 0, "故障必须真正命中");
        if growth {
            assert_eq!(count(3), 1, "不能读取原长度以外的探测字节");
        }
    }
}

#[test]
fn missing_and_empty_anchor_evidence_must_survive_until_final_revalidation() {
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("notes/empty")).unwrap();
    let reader = ReadOnlyVault::open(&f.root).unwrap();
    let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
    assert!(survey.paths().is_empty());
    let proof = reader
        .finish_scoped_observation(survey, &[path("notes/deep/missing")])
        .unwrap();
    f.write("notes/deep/missing", b"now present");
    assert!(reader.revalidate_scoped_observation(&proof).is_err());
    let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
    fs::remove_dir(f.root.join("notes/empty")).unwrap();
    assert!(reader.finish_scoped_observation(survey, &[]).is_err());
    let missing = Fixture::new();
    fs::create_dir(&missing.root).unwrap();
    let reader = ReadOnlyVault::open(&missing.root).unwrap();
    assert!(reader.begin_scoped_observation(&selection(), &[]).is_err());
}

#[test]
fn canonical_survey_preserves_instance_watermark_and_unknown_gate() {
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/a", b"same");
    let survey = broker.begin_sync_observation(&selection(), &[]).unwrap();
    drop(broker);
    let mut broker = f.reopen();
    reset();
    assert!(broker.finish_sync_observation(survey, &[]).is_err());
    assert_eq!(count(1), 0);
    let survey = broker.begin_sync_observation(&selection(), &[]).unwrap();
    broker
        .mutate(Mutation::create(
            path("outside"),
            b"new".to_vec(),
            "survey-watermark",
        ))
        .unwrap();
    reset();
    assert!(broker.finish_sync_observation(survey, &[]).is_err());
    assert_eq!(count(1), 0);
    let survey = broker.begin_sync_observation(&selection(), &[]).unwrap();
    broker.writer.fault.store(3, Ordering::Release);
    assert!(
        broker
            .mutate(Mutation::create(
                path("unknown"),
                b"new".to_vec(),
                "survey-unknown"
            ))
            .is_err()
    );
    reset();
    assert!(broker.finish_sync_observation(survey, &[]).is_err());
    assert_eq!(count(1), 0);
}

#[test]
fn canonical_final_union_rejects_new_excluded_or_case_alias_paths_before_body() {
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/a", b"a");
    for name in ["notes/TASKS.md", "notes/PRIVATE/a", "notes/A"] {
        let survey = broker.begin_sync_observation(&selection(), &[]).unwrap();
        reset();
        assert!(
            broker
                .finish_sync_observation(survey, &[path(name)])
                .is_err()
        );
        assert_eq!(count(1), 0);
    }
}
