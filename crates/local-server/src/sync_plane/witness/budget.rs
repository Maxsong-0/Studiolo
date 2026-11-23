//! 三方并集与同步预算比对（BUG-047）。路径条数：算出并集后、再观察 canonical 之前先比；
//! 比对内容的字节数：两侧观察都拿到后、交范围批准之前先比。用量都记进健康节；超了就按观察失败收工，
//! 不再往下走（写者与范围并集自己的检查留作兜底）。

use super::super::PlaneError;
use super::super::driver::diagnostics;
use super::super::health::PlaneHealth;
use studiolo_core::{SYNC_BYTE_BUDGET, SYNC_PATH_BUDGET};
use studiolo_host_config::SyncPathEvidence;

/// 记下这一轮三方并集的路径数；超过预算打一行固定日志、按观察失败收工（不分页、不截断）。
///
/// # Errors
/// 并集超过 [`SYNC_PATH_BUDGET`] 返回 `Observation`。
pub(super) fn check(health: &PlaneHealth, used: usize) -> Result<(), PlaneError> {
    let used_count = u64::try_from(used).unwrap_or(u64::MAX);
    let budget = SYNC_PATH_BUDGET as u64;
    health.record_path_usage(used_count, budget);
    if used > SYNC_PATH_BUDGET {
        diagnostics::path_budget_exceeded(used_count, budget);
        return Err(PlaneError::Observation);
    }
    Ok(())
}

/// 比对内容的字节合计：基线、canonical、镜像三组里每个「存在」的条目各算一遍，
/// 与 `SyncScope::approve_union` 的累计口径相同（同一个文件三方都有就算三遍）；溢出按饱和算。
pub(super) fn observed_bytes(groups: [&[SyncPathEvidence]; 3]) -> u64 {
    groups
        .into_iter()
        .flatten()
        .filter_map(|item| item.content)
        .fold(0, |total, content| total.saturating_add(content.bytes))
}

/// 记下这一轮比对内容的字节合计；超过预算打一行固定日志、按观察失败收工。
///
/// # Errors
/// 合计超过 [`SYNC_BYTE_BUDGET`] 返回 `Observation`。
pub(super) fn check_bytes(health: &PlaneHealth, used: u64) -> Result<(), PlaneError> {
    health.record_byte_usage(used, SYNC_BYTE_BUDGET);
    if used > SYNC_BYTE_BUDGET {
        diagnostics::byte_budget_exceeded(used, SYNC_BYTE_BUDGET);
        return Err(PlaneError::Observation);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{check, check_bytes, observed_bytes};
    use crate::sync_plane::PlaneError;
    use crate::sync_plane::health::{BridgeHealthProbe, PlaneHealth};
    use studiolo_core::{SYNC_BYTE_BUDGET, SYNC_PATH_BUDGET, VaultPath};
    use studiolo_host_config::{SyncContentMetadata, SyncFileKind, SyncPathEvidence};
    use studiolo_observability::health::BudgetUsage;

    fn entry(name: &str, bytes: Option<u64>) -> SyncPathEvidence {
        SyncPathEvidence {
            path: VaultPath::parse(name).expect("固定路径合法"),
            content: bytes.map(|bytes| SyncContentMetadata {
                kind: SyncFileKind::Markdown,
                bytes,
            }),
        }
    }

    #[test]
    fn union_within_budget_is_recorded_and_passes() {
        let health = PlaneHealth::new(None);
        assert!(check(&health, 4108).is_ok(), "169 现场的 4108 条在预算内");
        assert_eq!(
            health.bridge().path_usage,
            Some(BudgetUsage {
                used: 4108,
                budget: SYNC_PATH_BUDGET as u64,
            })
        );
        assert!(check(&health, SYNC_PATH_BUDGET).is_ok(), "恰好用满可以");
    }

    #[test]
    fn union_over_budget_is_recorded_and_stops_the_round() {
        let health = PlaneHealth::new(None);
        assert!(matches!(
            check(&health, SYNC_PATH_BUDGET + 1),
            Err(PlaneError::Observation)
        ));
        assert_eq!(
            health.bridge().path_usage,
            Some(BudgetUsage {
                used: SYNC_PATH_BUDGET as u64 + 1,
                budget: SYNC_PATH_BUDGET as u64,
            }),
            "超了也记下确切条数，健康页才说得出超了多少"
        );
    }

    #[test]
    fn observed_bytes_count_every_present_entry_once_per_group() {
        let baseline = [entry("a.md", Some(10)), entry("b.md", None)];
        let canonical = [entry("a.md", Some(20))];
        let mirror = [entry("a.md", Some(30)), entry("c.md", Some(5))];
        assert_eq!(
            observed_bytes([&baseline, &canonical, &mirror]),
            65,
            "同一个文件三方都有就算三遍，缺失不计"
        );
        assert_eq!(observed_bytes([&[], &[], &mirror]), 35);
        let huge = [entry("a.md", Some(u64::MAX)), entry("b.md", Some(1))];
        assert_eq!(observed_bytes([&huge, &[], &[]]), u64::MAX, "溢出按饱和算");
    }

    #[test]
    fn bytes_within_budget_are_recorded_and_pass() {
        let health = PlaneHealth::new(None);
        assert!(
            check_bytes(&health, 675_858_741).is_ok(),
            "169 现场约 676 MB"
        );
        assert_eq!(
            health.bridge().byte_usage,
            Some(BudgetUsage {
                used: 675_858_741,
                budget: SYNC_BYTE_BUDGET,
            })
        );
        assert!(
            check_bytes(&health, SYNC_BYTE_BUDGET).is_ok(),
            "恰好用满可以"
        );
        assert_eq!(health.bridge().path_usage, None, "不动路径用量");
    }

    #[test]
    fn bytes_over_budget_are_recorded_and_stop_the_round() {
        let health = PlaneHealth::new(None);
        assert!(matches!(
            check_bytes(&health, SYNC_BYTE_BUDGET + 1),
            Err(PlaneError::Observation)
        ));
        assert_eq!(
            health.bridge().byte_usage,
            Some(BudgetUsage {
                used: SYNC_BYTE_BUDGET + 1,
                budget: SYNC_BYTE_BUDGET,
            }),
            "超了也记下确切字节数"
        );
    }
}
