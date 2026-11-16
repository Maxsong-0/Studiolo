//! UTC 保留期和单调作业预算取更早者；时钟回退或到期后永不重新放行。
use crate::{ProcessError, Result};
use std::{
    cell::Cell,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use studiolo_core::checkpoint::InstantMillis;

pub(crate) struct Window {
    deadline: Cell<Instant>,
    last: Cell<InstantMillis>,
    expiry: InstantMillis,
    closed: Cell<bool>,
}
impl Window {
    pub fn new(
        deadline: Instant,
        started: Instant,
        now: InstantMillis,
        cutoff: InstantMillis,
        expiry: InstantMillis,
    ) -> Result<Self> {
        if deadline <= started || deadline > started + Duration::from_secs(7200) || cutoff > now {
            return Err(ProcessError::Deadline);
        }
        let window = Self {
            deadline: Cell::new(deadline),
            last: Cell::new(now),
            expiry,
            closed: Cell::new(false),
        };
        window.observe(now, started)?;
        Ok(window)
    }
    pub fn deadline(&self) -> Instant {
        self.deadline.get()
    }
    pub fn check(&self) -> Result<()> {
        self.current_time().map(|_| ())
    }
    pub fn current_time(&self) -> Result<InstantMillis> {
        // 先采单调时钟；读取墙钟耗时不会凭空延长有效期。
        let instant = Instant::now();
        match utc_now() {
            Ok(now) => {
                self.observe(now, instant)?;
                Ok(now)
            }
            Err(error) => {
                self.closed.set(true);
                Err(error)
            }
        }
    }
    fn observe(&self, now: InstantMillis, instant: Instant) -> Result<()> {
        if self.closed.get()
            || now < self.last.get()
            || now >= self.expiry
            || instant >= self.deadline.get()
        {
            self.closed.set(true);
            return Err(ProcessError::Deadline);
        }
        let remaining = self
            .expiry
            .millis()
            .checked_sub(now.millis())
            .and_then(|ms| u64::try_from(ms).ok())
            .and_then(|ms| instant.checked_add(Duration::from_millis(ms)));
        let Some(expiry) = remaining else {
            self.closed.set(true);
            return Err(ProcessError::Deadline);
        };
        self.last.set(now);
        self.deadline.set(self.deadline.get().min(expiry));
        Ok(())
    }
}
pub(crate) fn utc_now() -> Result<InstantMillis> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProcessError::Deadline)?
        .as_millis();
    Ok(InstantMillis::from_millis(
        i64::try_from(millis).map_err(|_| ProcessError::Deadline)?,
    ))
}

#[cfg(test)]
#[path = "../../tests/support/source_window.rs"]
mod tests;
