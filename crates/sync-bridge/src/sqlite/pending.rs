//! 仅内存的已验证索引，避免 N 条动作被每次查找或审计扫描成 N²。

use super::{BridgeError, model::*, receipts};
use std::collections::{BTreeMap, BTreeSet};

impl Pending {
    pub(super) fn new(
        input: crate::Input,
        plan: Box<crate::Plan>,
        actions: Vec<Progress>,
        summary_ack: Option<String>,
    ) -> Result<Self> {
        let cached_hash = receipts::input_hash(&input)?;
        let mut p = Self {
            input,
            cached_plan: plan,
            cached_hash,
            actions,
            summary_ack,
            index: BTreeMap::new(),
            intent_waiting: BTreeSet::new(),
            outcome_waiting: BTreeSet::new(),
            unknown_count: 0,
            failed_count: 0,
        };
        p.rebuild()?;
        Ok(p)
    }
    pub(super) fn rebuild(&mut self) -> Result<()> {
        if self.actions.len() != self.cached_plan.actions().len() {
            return Err(BridgeError::CorruptStore);
        }
        self.index = self
            .cached_plan
            .actions()
            .iter()
            .enumerate()
            .map(|(i, a)| (a.key.path.clone(), i))
            .collect();
        self.intent_waiting.clear();
        self.outcome_waiting.clear();
        self.unknown_count = 0;
        self.failed_count = 0;
        for (i, a) in self.actions.iter().enumerate() {
            if a.intent_ack.is_none() {
                self.intent_waiting.insert(i);
            }
            if a.receipt.is_some() && a.outcome_ack.is_none() {
                self.outcome_waiting.insert(i);
            }
            if a.receipt.is_none() {
                self.unknown_count += 1;
            }
            if failed(a) {
                self.failed_count += 1;
            }
        }
        Ok(())
    }
    pub(super) fn refresh(&mut self, i: usize, old: &Progress) -> Result<()> {
        let a = self.actions.get(i).ok_or(BridgeError::NotFound)?;
        if a.intent_ack.is_none() {
            self.intent_waiting.insert(i);
        } else {
            self.intent_waiting.remove(&i);
        }
        if a.receipt.is_some() && a.outcome_ack.is_none() {
            self.outcome_waiting.insert(i);
        } else {
            self.outcome_waiting.remove(&i);
        }
        self.unknown_count = self
            .unknown_count
            .checked_sub(usize::from(old.receipt.is_none()))
            .and_then(|n| n.checked_add(usize::from(a.receipt.is_none())))
            .ok_or(BridgeError::CorruptStore)?;
        self.failed_count = self
            .failed_count
            .checked_sub(usize::from(failed(old)))
            .and_then(|n| n.checked_add(usize::from(failed(a))))
            .ok_or(BridgeError::CorruptStore)?;
        Ok(())
    }
}
fn failed(a: &Progress) -> bool {
    matches!(
        a.receipt.as_ref().map(|r| &r.outcome),
        Some(EffectOutcome::NoEffect { .. })
    )
}
