//! 连接器的限频：纯逻辑，时刻由调用方传入。每个类型每个入口每分钟 12 次；同一（类型，原因）的拒绝每分钟只登记一条。
//!
//! 键只来自已认证的类型和固定的入口名，条数有界。
use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

const PER_MINUTE: usize = 12;
const MINUTE: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Limits {
    calls: BTreeMap<(String, &'static str), VecDeque<Instant>>,
    denials: BTreeMap<(String, &'static str), Instant>,
}

impl Limits {
    /// 放行就记一次；超了给出还要等几秒（至少 1 秒）。
    pub(super) fn admit(
        &mut self,
        agent_type: &str,
        route: &'static str,
        now: Instant,
    ) -> Result<(), u64> {
        let calls = self
            .calls
            .entry((agent_type.to_owned(), route))
            .or_default();
        while calls
            .front()
            .is_some_and(|time| now.saturating_duration_since(*time) >= MINUTE)
        {
            calls.pop_front();
        }
        match calls.front() {
            Some(oldest) if calls.len() >= PER_MINUTE => {
                let left = MINUTE.saturating_sub(now.saturating_duration_since(*oldest));
                Err(left.as_secs_f64().ceil().max(1.0) as u64)
            }
            _ => {
                calls.push_back(now);
                Ok(())
            }
        }
    }

    /// 这条拒绝要不要登记。
    pub(super) fn denial_due(
        &mut self,
        agent_type: &str,
        reason: &'static str,
        now: Instant,
    ) -> bool {
        self.denials
            .retain(|_, time| now.saturating_duration_since(*time) < MINUTE);
        let slot = (agent_type.to_owned(), reason);
        if self.denials.contains_key(&slot) {
            return false;
        }
        self.denials.insert(slot, now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twelve_calls_a_minute_per_type_and_route_with_the_wait_spelled_out() {
        let mut limits = Limits::default();
        let start = Instant::now();
        for second in 0..12 {
            assert_eq!(
                limits.admit("a", "board", start + Duration::from_secs(second)),
                Ok(())
            );
        }
        let at = |second| start + Duration::from_secs(second);
        assert_eq!(limits.admit("a", "board", at(12)), Err(48));
        // 别的入口、别的类型各算各的。
        assert_eq!(limits.admit("a", "task", at(12)), Ok(()));
        assert_eq!(limits.admit("b", "board", at(12)), Ok(()));
        // 被拒的不占名额；最早那次满一分钟就放一个进来。
        assert_eq!(
            limits.admit("a", "board", at(60) - Duration::from_millis(1)),
            Err(1)
        );
        assert_eq!(limits.admit("a", "board", at(60)), Ok(()));
        assert_eq!(limits.admit("a", "board", at(60)), Err(1));
    }

    #[test]
    fn the_same_denial_is_recorded_once_a_minute() {
        let mut limits = Limits::default();
        let start = Instant::now();
        assert!(limits.denial_due("a", "x", start));
        assert!(!limits.denial_due("a", "x", start + Duration::from_secs(59)));
        assert!(limits.denial_due("a", "y", start + Duration::from_secs(59)));
        assert!(limits.denial_due("b", "x", start + Duration::from_secs(59)));
        assert!(limits.denial_due("a", "x", start + Duration::from_secs(60)));
    }
}
