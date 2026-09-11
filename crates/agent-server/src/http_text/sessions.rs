//! HTTP 实例会话：单调时钟、有界名额与取消时自动释放的初始化预留。
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
struct Limits {
    total: usize,
    per_type: usize,
    starts_per_minute: usize,
    idle: Duration,
    absolute: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            total: 128,
            per_type: 32,
            starts_per_minute: 32,
            idle: Duration::from_secs(900),
            absolute: Duration::from_secs(3600),
        }
    }
}

struct Entry<T> {
    agent_type: String,
    created: Instant,
    last_used: Instant,
    value: Option<Arc<T>>,
}

impl<T> Entry<T> {
    fn expired(&self, now: Instant, limits: Limits) -> bool {
        self.value.is_some()
            && (now.duration_since(self.created) >= limits.absolute
                || now.duration_since(self.last_used) >= limits.idle)
    }
}

struct Table<T> {
    entries: HashMap<String, Entry<T>>,
    limits: Limits,
    window: Instant,
    starts: usize,
}

pub(super) struct Sessions<T> {
    inner: Arc<Mutex<Table<T>>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ReserveError {
    Full,
    Unavailable,
}

impl<T> Sessions<T> {
    pub(super) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Table {
                entries: HashMap::new(),
                limits: Limits::default(),
                window: Instant::now(),
                starts: 0,
            })),
        }
    }

    pub(super) fn reserve(
        &self,
        agent_type: &str,
        now: Instant,
    ) -> Result<Reservation<T>, ReserveError> {
        let mut table = self.inner.lock().map_err(|_| ReserveError::Unavailable)?;
        let limits = table.limits;
        table.entries.retain(|_, entry| !entry.expired(now, limits));
        if now.duration_since(table.window) >= Duration::from_secs(60) {
            table.window = now;
            table.starts = 0;
        }
        if table.entries.len() >= limits.total
            || table.starts >= limits.starts_per_minute
            || table
                .entries
                .values()
                .filter(|entry| entry.agent_type == agent_type)
                .count()
                >= limits.per_type
        {
            return Err(ReserveError::Full);
        }
        let id = loop {
            let candidate = ulid::Ulid::generate().to_string();
            if !table.entries.contains_key(&candidate) {
                break candidate;
            }
        };
        table.entries.insert(
            id.clone(),
            Entry {
                agent_type: agent_type.to_owned(),
                created: now,
                last_used: now,
                value: None,
            },
        );
        table.starts += 1;
        Ok(Reservation {
            inner: self.inner.clone(),
            id,
        })
    }

    /// 查找不续期；只有成功供给后 touch 才推进闲置期限。
    pub(super) fn get(&self, id: &str, agent_type: &str, now: Instant) -> Option<Arc<T>> {
        let mut table = self.inner.lock().ok()?;
        let entry = table.entries.get(id)?;
        if entry.expired(now, table.limits) {
            table.entries.remove(id);
            return None;
        }
        (entry.agent_type == agent_type)
            .then(|| entry.value.clone())
            .flatten()
    }

    pub(super) fn touch(&self, id: &str, agent_type: &str, now: Instant) -> bool {
        let Ok(mut table) = self.inner.lock() else {
            return false;
        };
        let limits = table.limits;
        let Some(entry) = table.entries.get_mut(id) else {
            return false;
        };
        if entry.agent_type != agent_type || entry.value.is_none() || entry.expired(now, limits) {
            return false;
        }
        entry.last_used = now;
        true
    }
}

/// 预留期间包含在总容量内；失败或 future 被取消时 Drop 清除预留。
pub(super) struct Reservation<T> {
    inner: Arc<Mutex<Table<T>>>,
    id: String,
}

impl<T> Reservation<T> {
    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn commit(self, value: Arc<T>, now: Instant) -> Result<String, ReserveError> {
        let mut table = self.inner.lock().map_err(|_| ReserveError::Unavailable)?;
        let limit = table.limits.absolute;
        let entry = table
            .entries
            .get_mut(&self.id)
            .ok_or(ReserveError::Unavailable)?;
        if now.duration_since(entry.created) >= limit {
            return Err(ReserveError::Unavailable);
        }
        entry.value = Some(value);
        entry.last_used = now;
        Ok(self.id.clone())
    }
}

impl<T> Drop for Reservation<T> {
    fn drop(&mut self) {
        if let Ok(mut table) = self.inner.lock()
            && table
                .entries
                .get(&self.id)
                .is_some_and(|entry| entry.value.is_none())
        {
            table.entries.remove(&self.id);
        }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
