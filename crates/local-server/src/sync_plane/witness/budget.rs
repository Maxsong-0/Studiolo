//! 三方并集与同步路径预算比对（BUG-047）：算出并集后、再观察 canonical 之前先比，
//! 用量记进健康节；超了就按观察失败收工，不再去调写者（写者自己的检查留作兜底）。

use super::super::PlaneError;
use super::super::driver::diagnostics;
use super::super::health::PlaneHealth;
use studiolo_core::SYNC_PATH_BUDGET;

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

#[cfg(test)]
mod tests {
    use super::check;
    use crate::sync_plane::PlaneError;
    use crate::sync_plane::health::{BridgeHealthProbe, PlaneHealth};
    use studiolo_core::SYNC_PATH_BUDGET;
    use studiolo_observability::health::PathUsage;

    #[test]
    fn union_within_budget_is_recorded_and_passes() {
        let health = PlaneHealth::new(None);
        assert!(check(&health, 4108).is_ok(), "169 现场的 4108 条在预算内");
        assert_eq!(
            health.bridge().path_usage,
            Some(PathUsage {
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
            Some(PathUsage {
                used: SYNC_PATH_BUDGET as u64 + 1,
                budget: SYNC_PATH_BUDGET as u64,
            }),
            "超了也记下确切条数，健康页才说得出超了多少"
        );
    }
}
