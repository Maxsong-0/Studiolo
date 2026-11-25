//! 拒绝登记的限频：纯逻辑，时刻由调用方传入。同一（名下，原因）每分钟只登记一条。
//!
//! 「名下」只许来自有界集合：已认证的类型名，或固定的入口名。请求里带来的东西（路径、令牌、地址）不许当名字，
//! 否则这张表和账本就又是谁都能撑大的。
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const MINUTE: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(crate) struct Denials {
    recorded: BTreeMap<(String, &'static str), Instant>,
}

impl Denials {
    /// 这条拒绝要不要登记；要就当场算用掉这一分钟。
    pub(crate) fn due(&mut self, slot: &str, reason: &'static str, now: Instant) -> bool {
        self.recorded
            .retain(|_, time| now.saturating_duration_since(*time) < MINUTE);
        let slot = (slot.to_owned(), reason);
        if self.recorded.contains_key(&slot) {
            return false;
        }
        self.recorded.insert(slot, now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_denial_is_recorded_once_a_minute() {
        let mut denials = Denials::default();
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        assert!(denials.due("a", "x", start));
        assert!(!denials.due("a", "x", at(59)));
        assert!(denials.due("a", "y", at(59)));
        assert!(denials.due("b", "x", at(59)));
        assert!(denials.due("a", "x", at(60)));
        // 没登记的那次不往后推时间：从登记那一刻起满一分钟就再登记。
        assert!(!denials.due("a", "x", at(119)));
        assert!(denials.due("a", "x", at(120)));
    }

    #[test]
    fn stale_entries_are_dropped() {
        let mut denials = Denials::default();
        let start = Instant::now();
        assert!(denials.due("a", "x", start));
        assert!(denials.due("b", "x", start + Duration::from_secs(60)));
        assert_eq!(denials.recorded.len(), 1);
    }
}
