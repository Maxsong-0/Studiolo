//! MirrorCAS 行为与边界契约；这里的上限是内部工程上限，不是任何服务端配额。

use super::ports::*;
use super::support::*;
use crate::executor::*;

fn take(cas: &mut MirrorCas, sink: &mut FakeSink, bytes: &[u8]) -> Result<ObjectId, CasFault> {
    cas.begin_receive(bytes.len() as u64, None, sink)?;
    for frame in bytes.chunks(cas.limits().frame_bytes) {
        cas.accept_frame(frame, sink)?;
    }
    cas.finish_receive(sink)
}

#[test]
fn internal_ceilings_are_the_frozen_engineering_numbers_not_a_service_quota() {
    let limits = CasLimits::internal();
    assert_eq!(limits.max_object_bytes, 256 * 1024 * 1024);
    assert_eq!(limits.max_stage_bytes, 512 * 1024 * 1024);
    assert_eq!(limits.max_stage_entries, 64);
    assert_eq!(limits.max_receiving, 1);
    assert_eq!(limits.max_prepared, 1);
    assert_eq!(limits.frame_bytes, 64 * 1024);
}

#[test]
fn objects_are_addressed_by_their_actual_content_and_the_same_bytes_reuse_one_entry() {
    let mut cas = MirrorCas::new(CasLimits::internal());
    let mut sink = FakeSink::new(log());
    let first = take(&mut cas, &mut sink, body(2)).unwrap();
    let second = take(&mut cas, &mut sink, body(2)).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.digest(), version(2).sha256);
    let usage = cas.usage();
    assert_eq!(usage.entries, 1);
    assert_eq!(usage.bytes, version(2).bytes);
    // 尚未被引用的对象一律计入孤儿占用。
    assert_eq!(usage.orphans, 1);
    assert_eq!(usage.orphan_bytes, version(2).bytes);
    assert_eq!(cas.orphans(), vec![first]);
}

#[test]
fn a_single_object_beyond_the_frozen_object_ceiling_is_refused_before_any_byte_moves() {
    let mut cas = MirrorCas::new(CasLimits::internal());
    let mut sink = FakeSink::new(log());
    assert_eq!(
        cas.begin_receive(256 * 1024 * 1024 + 1, None, &mut sink),
        Err(CasFault::ObjectTooLarge)
    );
    assert_eq!(cas.usage().entries, 0);
    assert!(sink.finished.is_empty());
}

#[test]
fn the_sixty_fifth_stage_entry_is_refused_and_orphans_still_occupy_the_budget() {
    let mut cas = MirrorCas::new(CasLimits::internal());
    let mut sink = FakeSink::new(log());
    let mut staged = Vec::new();
    for n in 0..64_u32 {
        staged.push(take(&mut cas, &mut sink, &n.to_be_bytes()).unwrap());
    }
    assert_eq!(cas.usage().entries, 64);
    assert_eq!(cas.usage().orphans, 64);
    assert_eq!(
        cas.begin_receive(4, None, &mut sink),
        Err(CasFault::StageFull)
    );
    // 已在账本里的地址不再占新条目，因此仍可重新落库同一内容。
    let known = *staged.first().unwrap();
    assert!(cas.contains(known));
    cas.begin_receive(4, Some(known), &mut sink).unwrap();
    cas.abort_receive(&mut sink).unwrap();
}

#[test]
fn the_stage_byte_ceiling_counts_orphans_and_refuses_the_projected_total() {
    let mut cas = MirrorCas::new(CasLimits {
        max_stage_bytes: 8,
        ..CasLimits::internal()
    });
    let mut sink = FakeSink::new(log());
    take(&mut cas, &mut sink, &[1, 2, 3, 4]).unwrap();
    take(&mut cas, &mut sink, &[5, 6, 7, 8]).unwrap();
    assert_eq!(cas.usage().bytes, 8);
    assert_eq!(
        cas.begin_receive(1, None, &mut sink),
        Err(CasFault::StageBytesExceeded)
    );
}

#[test]
fn a_frame_larger_than_the_fixed_buffer_is_refused() {
    let mut cas = MirrorCas::new(CasLimits::internal());
    let mut sink = FakeSink::new(log());
    let oversized = vec![0_u8; 64 * 1024 + 1];
    cas.begin_receive(oversized.len() as u64, None, &mut sink)
        .unwrap();
    assert_eq!(
        cas.accept_frame(&oversized, &mut sink),
        Err(CasFault::FrameTooLarge)
    );
    cas.abort_receive(&mut sink).unwrap();
    assert_eq!(cas.usage().entries, 0);
}

#[test]
fn only_one_object_may_be_received_or_prepared_at_a_time() {
    let mut cas = MirrorCas::new(CasLimits::internal());
    let mut sink = FakeSink::new(log());
    cas.begin_receive(4, None, &mut sink).unwrap();
    assert_eq!(
        cas.begin_receive(4, None, &mut sink),
        Err(CasFault::ReceiveInProgress)
    );
    cas.accept_frame(&[1, 2, 3, 4], &mut sink).unwrap();
    let first = cas.finish_receive(&mut sink).unwrap();
    let second = take(&mut cas, &mut sink, &[5, 6, 7, 8]).unwrap();
    cas.prepare(first).unwrap();
    assert_eq!(cas.prepare(second), Err(CasFault::PreparedInUse));
    assert_eq!(cas.forget(first), Err(CasFault::StillReferenced));
    assert_eq!(cas.release_prepared(false).unwrap(), first);
    assert_eq!(cas.release_prepared(false), Err(CasFault::NoPrepared));
    cas.forget(first).unwrap();
    assert!(!cas.contains(first));
}

#[test]
fn content_that_does_not_hash_to_the_expected_address_is_never_admitted() {
    let mut cas = MirrorCas::new(CasLimits::internal());
    let mut sink = FakeSink::new(log());
    let expected = ObjectId::from_digest(version(3).sha256);
    cas.begin_receive(4, Some(expected), &mut sink).unwrap();
    cas.accept_frame(&[9, 9, 9, 9], &mut sink).unwrap();
    assert_eq!(
        cas.finish_receive(&mut sink),
        Err(CasFault::AddressMismatch)
    );
    assert_eq!(cas.usage().entries, 0);
    assert!(sink.finished.is_empty());
    assert!(log_contains(&sink, "sink.discard"));
}

#[test]
fn a_stream_shorter_or_longer_than_the_declared_size_leaves_no_half_object() {
    let mut cas = MirrorCas::new(CasLimits::internal());
    let mut sink = FakeSink::new(log());
    cas.begin_receive(8, None, &mut sink).unwrap();
    cas.accept_frame(&[1, 2, 3, 4], &mut sink).unwrap();
    assert_eq!(
        cas.finish_receive(&mut sink),
        Err(CasFault::DeclaredBytesMismatch)
    );
    assert_eq!(cas.usage().entries, 0);
    cas.begin_receive(4, None, &mut sink).unwrap();
    assert_eq!(
        cas.accept_frame(&[1, 2, 3, 4, 5], &mut sink),
        Err(CasFault::DeclaredBytesMismatch)
    );
    cas.abort_receive(&mut sink).unwrap();
    assert_eq!(cas.usage().entries, 0);
}

fn log_contains(sink: &FakeSink, needle: &str) -> bool {
    sink.log.borrow().iter().any(|line| line == needle)
}
