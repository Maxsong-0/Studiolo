//! 小更新只备份对应行以便事务拒绝恢复内存；完整复制仅在 reserve/settle。
use super::{BridgeError, model::*};

pub(super) enum Undo {
    Full(Box<State>),
    Action(u64, usize, Box<Progress>),
    Summary(Option<String>),
    Retirement(Option<Retirement>),
    Fuse(Option<String>),
}
impl Undo {
    pub fn capture(s: &State, e: Edit) -> Result<Self> {
        Ok(match e {
            Edit::Full => Self::Full(Box::new(s.clone())),
            Edit::Action { sequence, index } => Self::Action(
                sequence,
                index,
                Box::new(
                    selected(s, sequence)?
                        .actions
                        .get(index)
                        .ok_or(BridgeError::NotFound)?
                        .clone(),
                ),
            ),
            Edit::Summary => Self::Summary(
                s.pending
                    .as_ref()
                    .ok_or(BridgeError::NotFound)?
                    .summary_ack
                    .clone(),
            ),
            Edit::Retirement => Self::Retirement(s.retirement.clone()),
            Edit::FuseAck => Self::Fuse(s.fuse.as_ref().ok_or(BridgeError::NotFound)?.ack.clone()),
        })
    }
    pub fn restore(self, s: &mut State) {
        match self {
            Self::Full(old) => *s = *old,
            Self::Action(seq, i, old) => {
                if let Some(p) = s
                    .pending
                    .iter_mut()
                    .chain(s.last.iter_mut())
                    .find(|p| p.input.plan_sequence == seq)
                {
                    if let Some(a) = p.actions.get_mut(i) {
                        *a = *old;
                    }
                    let _ = p.rebuild();
                }
            }
            Self::Summary(a) => {
                if let Some(p) = s.pending.as_mut() {
                    p.summary_ack = a;
                }
            }
            Self::Retirement(r) => s.retirement = r,
            Self::Fuse(a) => {
                if let Some(f) = s.fuse.as_mut() {
                    f.ack = a;
                }
            }
        }
    }
    pub fn refresh(&self, s: &mut State) -> Result<()> {
        if let Self::Action(seq, i, old) = self {
            s.pending
                .iter_mut()
                .chain(s.last.iter_mut())
                .find(|p| p.input.plan_sequence == *seq)
                .ok_or(BridgeError::NotFound)?
                .refresh(*i, old)?;
        }
        Ok(())
    }
}
pub(super) fn selected(s: &State, seq: u64) -> Result<&Pending> {
    s.pending
        .iter()
        .chain(s.last.iter())
        .find(|p| p.input.plan_sequence == seq)
        .ok_or(BridgeError::NotFound)
}
pub(super) fn validate(s: &State, e: Edit) -> Result<()> {
    match e {
        Edit::Full => super::validate::state(s),
        Edit::Action { sequence, index } => {
            super::validate::progress(selected(s, sequence)?, index)
        }
        Edit::Summary => {
            super::audit::check_summary(s.pending.as_ref().ok_or(BridgeError::NotFound)?)
        }
        Edit::Retirement => {
            if let Some(r) = &s.retirement {
                let last = s.last.as_ref().ok_or(BridgeError::NotFound)?;
                if r.old_sequence != last.input.plan_sequence
                    || r.old_floor != s.floor
                    || r.old_digest != super::audit::terminal_digest(last)?
                {
                    return Err(BridgeError::CorruptStore);
                }
                if let Some(ack) = &r.ack {
                    super::audit::check_ack(
                        ack,
                        &super::audit::event(s, super::audit::Slot::Retirement)?,
                    )?;
                }
            }
            Ok(())
        }
        Edit::FuseAck => {
            if let Some(ack) = s.fuse.as_ref().ok_or(BridgeError::NotFound)?.ack.as_ref() {
                super::audit::check_ack(ack, &super::audit::event(s, super::audit::Slot::Fuse)?)?;
            }
            Ok(())
        }
    }
}
