//! 保守删除熔断的整批边界契约：等号可过、越线整批阻断、溢出拒绝。

use super::failure::drive;
use super::runner::*;
use crate::executor::*;
use crate::*;

#[test]
fn exactly_five_deletions_pass_the_count_threshold_and_six_block_the_whole_plan() {
    let allowed = forged_deletions(200, 5, true);
    let (result, journal, _) = drive(&allowed);
    let report = result.unwrap();
    assert_eq!(report.deletion.deletions, 5);
    assert_eq!(report.deletion.verdict, None);
    assert_eq!(journal.issued.len(), 5);

    let blocked = forged_deletions(200, 6, true);
    let (result, journal, rig) = drive(&blocked);
    assert_eq!(
        result,
        Err(ExecutorError::DeletionFuseBlocked(FuseReason::Count))
    );
    assert!(journal.issued.is_empty());
    assert!(journal.outcomes.is_empty());
    // 整批阻断发生在任何正文被打开之前。
    assert_eq!(rig.first("open"), None);
    assert_eq!(rig.cas.usage().entries, 0);
}

#[test]
fn exactly_ten_percent_passes_the_ratio_threshold_and_one_more_blocks_the_whole_plan() {
    let allowed = forged_deletions(20, 2, true);
    let (result, journal, _) = drive(&allowed);
    assert_eq!(result.unwrap().deletion.verdict, None);
    assert_eq!(journal.issued.len(), 2);

    let blocked = forged_deletions(20, 3, true);
    let (result, journal, _) = drive(&blocked);
    assert_eq!(
        result,
        Err(ExecutorError::DeletionFuseBlocked(FuseReason::Ratio))
    );
    assert!(journal.issued.is_empty());
}

#[test]
fn a_baseline_with_zero_surviving_paths_blocks_any_deletion_at_all() {
    let plan = forged_deletions(4, 1, false);
    assert_eq!(audit_batch(&plan).unwrap().baseline_live_paths, 0);
    let (result, journal, _) = drive(&plan);
    assert_eq!(
        result,
        Err(ExecutorError::DeletionFuseBlocked(FuseReason::Ratio))
    );
    assert!(journal.issued.is_empty());
}

#[test]
fn the_deletion_threshold_arithmetic_refuses_integer_overflow_instead_of_wrapping() {
    let limits = Limits::conservative(1000).deletion;
    for stats in [
        DeleteStats {
            canonical: u64::MAX,
            mirror: 1,
            ..DeleteStats::default()
        },
        DeleteStats {
            canonical: u64::MAX / 10000 + 1,
            ..DeleteStats::default()
        },
        DeleteStats {
            mirror: 1,
            baseline_live_paths: u64::MAX,
            ..DeleteStats::default()
        },
    ] {
        assert_eq!(
            crate::executor::deletion::verdict(stats, limits),
            Err(ExecutorError::Overflow)
        );
    }
}
