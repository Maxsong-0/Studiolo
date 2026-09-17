//! 预占、身份与期限共用一个有界表；过期项须等实际 worker 关闭后才释放容量。
use super::binding::Binding;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub total: usize,
    pub per_type: usize,
    pub starts_per_minute: usize,
    pub idle: Duration,
    pub absolute: Duration,
    pub initialize: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            total: 128,
            per_type: 32,
            starts_per_minute: 32,
            idle: Duration::from_secs(900),
            absolute: Duration::from_secs(3600),
            initialize: Duration::from_secs(30),
        }
    }
}

struct Table {
    entries: HashMap<String, Arc<Binding>>,
    window: Instant,
    starts: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReserveError {
    Full,
    Unavailable,
}

pub(super) struct Pool {
    pub limits: Limits,
    inner: Mutex<Table>,
    work: Arc<tokio::sync::Semaphore>,
}

impl Pool {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            work: Arc::new(tokio::sync::Semaphore::new(16)),
            inner: Mutex::new(Table {
                entries: HashMap::new(),
                window: Instant::now(),
                starts: 0,
            }),
        }
    }

    pub fn reserve(
        &self,
        owner: &str,
        credential: &str,
        now: Instant,
    ) -> Result<Arc<Binding>, ReserveError> {
        let mut table = self.inner.lock().map_err(|_| ReserveError::Unavailable)?;
        if now.duration_since(table.window) >= Duration::from_secs(60) {
            table.window = now;
            table.starts = 0;
        }
        if table.entries.len() >= self.limits.total
            || table
                .entries
                .values()
                .filter(|entry| entry.owner == owner)
                .count()
                >= self.limits.per_type
            || table.starts >= self.limits.starts_per_minute
        {
            return Err(ReserveError::Full);
        }
        let key = ulid::Ulid::generate().to_string();
        let binding = Arc::new(Binding::new(
            key.clone(),
            owner,
            credential,
            now,
            self.limits,
            self.work.clone(),
        ));
        table.entries.insert(key, binding.clone());
        table.starts += 1;
        Ok(binding)
    }

    pub fn find(&self, session: &str) -> Option<Arc<Binding>> {
        self.inner
            .lock()
            .ok()?
            .entries
            .values()
            .find(|entry| entry.session.get().is_some_and(|id| id.as_ref() == session))
            .cloned()
    }

    pub fn all(&self) -> Vec<Arc<Binding>> {
        self.inner
            .lock()
            .map(|table| table.entries.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn release(&self, binding: &Binding) {
        if let Ok(mut table) = self.inner.lock() {
            table.entries.remove(&binding.key);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/mcp_transport_private/pool.rs"]
mod tests;
