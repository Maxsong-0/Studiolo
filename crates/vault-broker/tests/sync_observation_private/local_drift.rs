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
        let result = reader.finish_scoped_observation(survey, &[path("notes/new")]);
        assert!(
            matches!(result, Err(BrokerError::CaptureChanged)),
            "attack {attack}: {:?}",
            result.err()
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
        assert!(
            matches!(
                reader.finish_scoped_observation(survey, &[]),
                Err(BrokerError::CaptureChanged)
            ),
            "phase {phase}, growth {growth}"
        );
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
    assert!(matches!(
        reader.revalidate_scoped_observation(&proof),
        Err(BrokerError::CaptureChanged)
    ));
    let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
    fs::remove_dir(f.root.join("notes/empty")).unwrap();
    assert!(matches!(
        reader.finish_scoped_observation(survey, &[]),
        Err(BrokerError::CaptureChanged)
    ));
    let missing = Fixture::new();
    fs::create_dir(&missing.root).unwrap();
    let reader = ReadOnlyVault::open(&missing.root).unwrap();
    assert!(matches!(
        reader.begin_scoped_observation(&selection(), &[]),
        Err(BrokerError::NotFound(anchor)) if anchor == path("notes")
    ));
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
    assert!(matches!(
        broker.finish_sync_observation(survey, &[]),
        Err(BrokerError::CaptureChanged)
    ));
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
    assert!(matches!(
        broker.finish_sync_observation(survey, &[]),
        Err(BrokerError::CaptureChanged)
    ));
    assert_eq!(count(1), 0);
    let survey = broker.begin_sync_observation(&selection(), &[]).unwrap();
    broker.writer.fault.store(3, Ordering::Release);
    assert!(matches!(
        broker.mutate(Mutation::create(
            path("unknown"),
            b"new".to_vec(),
            "survey-unknown"
        )),
        Err(BrokerError::Io {
            operation: "injected writer interruption",
            ..
        })
    ));
    reset();
    assert!(matches!(
        broker.finish_sync_observation(survey, &[]),
        Err(BrokerError::MutationUnresolved)
    ));
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
        let result = broker.finish_sync_observation(survey, &[path(name)]);
        // 保留文件名按非法补充路径拒绝；大小写别名与已入索引的原拼写碰撞。
        let rejected = match name {
            "notes/TASKS.md" => {
                matches!(&result, Err(BrokerError::InvalidArtifact(reason)) if reason == INVALID)
            }
            "notes/PRIVATE/a" => matches!(
                &result,
                Err(BrokerError::PathCollision { existing, .. }) if existing == "notes/private"
            ),
            _ => matches!(
                &result,
                Err(BrokerError::PathCollision { existing, .. }) if existing == "notes/a"
            ),
        };
        assert!(rejected, "{name}: {:?}", result.err());
        assert_eq!(count(1), 0);
    }
}
