use super::*;
#[test]
fn changed_source_target_stage_and_root_never_borrow_a_sealed_ticket() {
    for attack in 0..4 {
        let f = Fixture::new();
        let mut b = f.open();
        let source = f.bytes(b"hello");
        let request = request(&source, "first", Precondition::MustNotExist);
        let ticket = b
            .prepare_stream_mutation(request, &source, limits(), metadata())
            .unwrap();
        match attack {
            0 => {
                fs::write(f.source.join("input.bin"), b"other").unwrap();
            }
            1 => {
                fs::write(f.root.join(path().as_str()), b"outside").unwrap();
            }
            2 => {
                fs::write(&stage_files(&f)[0], b"other").unwrap();
            }
            _ => {
                fs::rename(&f.root, f.root.with_extension("old")).unwrap();
                fs::create_dir(&f.root).unwrap();
            }
        }
        let result = b.commit_stream_mutation(ticket);
        if attack == 0 {
            // 已封存后来源不再是安装权威；仍只安装原封存字节。
            result.unwrap();
            assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"hello");
        } else {
            assert!(result.is_err());
        }
    }
}
#[test]
fn cross_instance_ticket_and_stream_as_old_mutation_replay_are_refused() {
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.bytes(b"hello");
    let request = request(&source, "first", Precondition::MustNotExist);
    let ticket = b
        .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
        .unwrap();
    drop(b);
    let mut b = f.reopen();
    assert!(matches!(
        b.cancel_stream_mutation(ticket),
        Err(BrokerError::CaptureChanged)
    ));
    assert!(matches!(
        b.mutation_replay_recorded(
            &Mutation::create(path(), b"hello".to_vec(), "first"),
            &metadata()
        ),
        Err(BrokerError::IdempotencyConflict)
    ));
    assert_eq!(b.write_status().unwrap().phase, WriterPhase::Prepared);
}
#[test]
fn replay_is_bound_to_actor_kind_bytes_and_path_but_not_present_source() {
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.bytes(b"hello");
    let request = request(&source, "first", Precondition::MustNotExist);
    let ticket = b
        .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
        .unwrap();
    let receipt = b.commit_stream_mutation(ticket).unwrap();
    fs::remove_file(f.source.join("input.bin")).unwrap();
    fs::remove_file(f.root.join(path().as_str())).unwrap();
    assert_eq!(
        b.stream_mutation_replay(&request, &metadata()).unwrap(),
        Some(receipt)
    );
    let mut changed = request.clone();
    changed.content.bytes += 1;
    assert!(b.stream_mutation_replay(&changed, &metadata()).is_err());
    let other = MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace).unwrap();
    assert!(b.stream_mutation_replay(&request, &other).is_err());
}
