use super::*;

pub(super) fn damage(f: &Fixture, revision: Revision, kind: u8) {
    let target = f.data.join("history").join(revision.to_string());
    match kind {
        0 => fs::remove_file(target).unwrap(),
        1 => {
            // 不改长度的原 inode 写入，不能只比较长度或目录 inode。
            let mut body = fs::read(&target).unwrap();
            body[0] ^= 0xff;
            fs::write(target, body).unwrap();
        }
        _ => {
            // 原对象尚在时先建新 inode，避免删除后 inode 重用掩盖替换。
            let replacement = f.source.join("same-content-replacement.bin");
            fs::write(&replacement, fs::read(&target).unwrap()).unwrap();
            fs::rename(replacement, target).unwrap();
        }
    }
}

#[test]
fn old_and_new_l0_drift_between_prepare_and_commit_preserves_canonical() {
    for previous in [false, true] {
        for kind in 0..3 {
            let f = Fixture::new();
            let mut b = f.open();
            let old = b
                .mutate(Mutation::create(path(), b"old".to_vec(), "old"))
                .unwrap();
            let source = f.bytes(b"new");
            let request = request(&source, "next", Precondition::Match(old.revision));
            let ticket = b
                .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
                .unwrap();
            assert_eq!(ticket.history.object_count(), 2);
            let revision = if previous {
                old.revision
            } else {
                ticket.target_revision()
            };
            damage(&f, revision, kind);
            assert!(b.commit_stream_mutation(ticket).is_err());
            assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"old");
            assert_eq!(b.write_status().unwrap().phase, WriterPhase::Prepared);
            assert_eq!(b.write_status().unwrap().committed_through, 1);
            assert!(
                b.stream_mutation_replay(&request, &metadata())
                    .unwrap()
                    .is_none()
            );
            drop(b);
            assert_eq!(
                f.reopen().write_status().unwrap().phase,
                WriterPhase::Prepared
            );
        }
    }
}

#[test]
fn l0_drift_at_final_stream_rename_check_leaves_installing_without_overwrite() {
    for point in [82, 83] {
        let f = Fixture::new();
        let mut b = f.open();
        let old = b
            .mutate(Mutation::create(path(), b"old".to_vec(), "old"))
            .unwrap();
        let source = f.bytes(b"new");
        let request = request(&source, "next", Precondition::Match(old.revision));
        let ticket = b
            .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
            .unwrap();
        fault(&b, point);
        assert!(matches!(
            b.commit_stream_mutation(ticket),
            Err(BrokerError::MutationUnresolved)
        ));
        assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"old");
        assert_eq!(b.write_status().unwrap().phase, WriterPhase::Installing);
        assert_eq!(stage_files(&f).len(), 1);
        drop(b);
        let b = f.reopen();
        assert_eq!(b.write_status().unwrap().phase, WriterPhase::Installing);
        assert!(
            b.stream_mutation_replay(&request, &metadata())
                .unwrap()
                .is_none()
        );
        assert_eq!(b.write_status().unwrap().committed_through, 1);
    }
}

#[test]
fn create_only_also_requires_its_new_l0_proof() {
    for kind in 0..3 {
        let f = Fixture::new();
        let mut b = f.open();
        let source = f.bytes(b"new");
        let ticket = b
            .prepare_stream_mutation(
                request(&source, "create", Precondition::MustNotExist),
                &source,
                limits(),
                metadata(),
            )
            .unwrap();
        assert_eq!(ticket.history.object_count(), 1);
        damage(&f, ticket.target_revision(), kind);
        assert!(b.commit_stream_mutation(ticket).is_err());
        assert!(!f.root.join(path().as_str()).exists());
        assert_eq!(b.write_status().unwrap().phase, WriterPhase::Prepared);
        assert_eq!(b.write_status().unwrap().committed_through, 0);
    }
}
