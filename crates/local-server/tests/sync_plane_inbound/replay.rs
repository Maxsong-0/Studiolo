//! 原 Broker 回执仍在、上层尚未记录时，stage 重开且镜像漂移也能重建同一指纹。

use super::fixture::{Fixture, NOW, SEQUENCE, Source, path, version};
use std::fs;
use studiolo_local_server::sync_plane::driver::content::inbound::{freeze_source, write_preserved};
use studiolo_local_server::sync_plane::driver::content::{
    BridgePrecondition, INLINE_MAX_BYTES, MirrorWriteError,
};
use studiolo_local_server::sync_plane::driver::reconcile::replay_outcome;
use studiolo_local_server::sync_plane::job_backend::{SeamVerdict, first_attempt_outcome};
use studiolo_sync_bridge::executor::{JobOutcome, NoEffectCause, ObjectId};

async fn lost_receipt(bytes: Vec<u8>, create: bool, mirror_missing: bool) {
    let f = Fixture::new(&bytes);
    let version = version(&bytes);
    let precondition = if create {
        fs::remove_file(f.root.join("vault/notes/body.txt")).unwrap();
        BridgePrecondition::MustNotExist
    } else {
        BridgePrecondition::Match(f.previous)
    };
    let mut stage = f.stage();
    f.freeze(&mut stage, &version);
    let owner = f.writer().await;
    let key = "inbound-original-key";
    let receipt = write_preserved(&owner, &stage, &path(), &version, precondition, key).unwrap();
    assert_eq!(
        fs::read(f.root.join("vault/notes/body.txt")).unwrap(),
        bytes
    );
    let commits = f.commits();
    assert_eq!(commits, 1);
    // 丢失上层回执：未写 Bridge 终态，关闭真实写者和 stage，再重开两份持久状态。
    owner.shutdown().await.unwrap();
    drop(owner);
    drop(stage);
    if mirror_missing {
        fs::remove_dir_all(f.root.join("mirror")).unwrap();
    } else {
        fs::write(
            f.root.join("mirror/notes/body.txt"),
            b"mirror changed later",
        )
        .unwrap();
    }
    // 目标也已变：能拿回原 revision 且不覆盖当前目标，才证明命中原键。
    fs::write(
        f.root.join("vault/notes/body.txt"),
        b"canonical changed later",
    )
    .unwrap();
    let stage = f.stage();
    let owner = f.writer().await;
    let replayed = write_preserved(&owner, &stage, &path(), &version, precondition, key).unwrap();
    assert_eq!(replayed, receipt);
    assert_eq!(f.commits(), commits, "重放不产生第二次安装");
    assert_eq!(
        fs::read(f.root.join("vault/notes/body.txt")).unwrap(),
        b"canonical changed later"
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn inline_create_and_replace_reopen_after_mirror_changed_or_disappeared() {
    for create in [true, false] {
        for missing in [false, true] {
            lost_receipt(b"frozen inline source".to_vec(), create, missing).await;
        }
    }
}

#[tokio::test]
async fn stream_create_and_replace_reopen_after_mirror_changed_or_disappeared() {
    for create in [true, false] {
        for missing in [false, true] {
            lost_receipt(vec![b'x'; INLINE_MAX_BYTES as usize + 1], create, missing).await;
        }
    }
}

#[tokio::test]
async fn damaged_or_missing_stage_keeps_unknown_even_when_mirror_has_original_bytes() {
    for large in [false, true] {
        for missing in [false, true] {
            let bytes = vec![
                b'x';
                if large {
                    INLINE_MAX_BYTES as usize + 1
                } else {
                    32
                }
            ];
            let f = Fixture::new(&bytes);
            let version = version(&bytes);
            let mut stage = f.stage();
            f.freeze(&mut stage, &version);
            let owner = f.writer().await;
            let precondition = BridgePrecondition::Match(f.previous);
            write_preserved(
                &owner,
                &stage,
                &path(),
                &version,
                precondition,
                "damaged-stage",
            )
            .unwrap();
            let object = stage.object_path(ObjectId::from_digest(version.sha256));
            drop(stage);
            if missing {
                fs::remove_file(object).unwrap();
            } else {
                fs::write(object, vec![b'y'; bytes.len()]).unwrap();
            }
            let stage = f.stage();
            assert_eq!(
                write_preserved(
                    &owner,
                    &stage,
                    &path(),
                    &version,
                    precondition,
                    "damaged-stage"
                ),
                Err(MirrorWriteError::Source)
            );
            assert_eq!(
                replay_outcome(SeamVerdict::SourceUnavailable, true, "damaged-stage"),
                JobOutcome::Unknown
            );
            assert_eq!(f.commits(), 1);
            owner.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn first_missing_or_changed_source_and_failed_index_never_call_broker() {
    for failure in ["missing", "digest", "length", "index"] {
        let f = Fixture::new(b"frozen");
        let frozen = version(b"frozen");
        let mut stage = f.stage();
        let mirror = f.root.join("mirror/notes/body.txt");
        match failure {
            "missing" => fs::remove_file(mirror).unwrap(),
            "digest" => fs::write(mirror, b"change").unwrap(),
            "length" => fs::write(mirror, b"longer source").unwrap(),
            "index" => fs::create_dir(f.root.join("plane/stage/index.json.new")).unwrap(),
            _ => unreachable!(),
        }
        let owner = f.writer().await;
        let mut source = Source::new(f.root.join("mirror"));
        let result = freeze_source(&mut stage, &mut source, &path(), &frozen, SEQUENCE, NOW)
            .map_err(|_| MirrorWriteError::Source)
            .and_then(|()| {
                write_preserved(
                    &owner,
                    &stage,
                    &path(),
                    &frozen,
                    BridgePrecondition::Match(f.previous),
                    "never-issued",
                )
            });
        assert_eq!(result, Err(MirrorWriteError::Source), "{failure}");
        assert!(matches!(
            first_attempt_outcome(SeamVerdict::SourceUnavailable, "never-issued"),
            JobOutcome::NoEffect {
                reason: NoEffectCause::CancelledBeforeInstall,
                ..
            }
        ));
        assert_eq!(f.commits(), 0);
        assert_eq!(
            fs::read_dir(f.root.join("data/idempotency"))
                .unwrap()
                .count(),
            0
        );
        owner.shutdown().await.unwrap();
    }
}
