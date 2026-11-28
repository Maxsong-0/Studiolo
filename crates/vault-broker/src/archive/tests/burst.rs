//! 7 天内的突发（T-249 G4，用户 2026-11-27 定）：散件到 95% 起，已在成功封存之内、不满 7 天的也打；
//! 没到 95% 照旧要满 7 天，没进过成功封存的一条不碰。
use super::*;

/// 一天前写的 `count` 条，加一篇刚写的；现在打一轮能打几包。
fn packed_now(count: usize, sealed_offset: u64) -> usize {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    bulk_with(&broker, 0..count, Shape::young());
    let cutoff = ArchiveCutoff {
        sealed_offset,
        now_ms: now(),
        ..later()
    };
    pack(&mut broker, cutoff)
}

/// 4096 的 95% 是 3891.2：3892 条起算到了。到了就连同刚写的一起打；差一条不打；没封存的不打。
#[test]
fn young_records_are_packed_only_from_ninety_five_percent_and_only_if_sealed() {
    assert_eq!(packed_now(3890, 1_000_000), 0);
    assert_eq!(packed_now(3891, 1_000_000), 3892);
    assert_eq!(packed_now(3891, 0), 0);
}
