//! 人侧起来后自己把账本读到末尾，不靠外面的请求一页一页推（BUG-042）。
//!
//! 队列的进度只在内存里：人侧每次启动都从账本第一条读起。以前只有 worker、定时器、健康页的请求推它
//! 往前读，一条请求最多读 `SCAN_BUDGET`，账本有 70 万条时头十几分钟一条请求都答不上来。现在监听一起来
//! 就在后台线程里把它读完（`service.rs`），请求那条路一个字不变：还没读完时请求照旧自己接着读。
use super::{BackupProducer, PAGE, TARGET};
use std::sync::atomic::{AtomicBool, Ordering};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

impl BackupProducer {
    /// 读到账本末尾或被叫停为止；每读一页之前看一次停止标志，关停最多等手上这一页读完。
    ///
    /// 账本在状态锁之外读，拿锁只做内存里的记账：请求和健康页等这把锁，等不到它头上。
    /// 读账本出错就收手，留给之后的请求照旧自己读。
    pub fn catch_up(&self, stop: &AtomicBool) {
        while !stop.load(Ordering::Acquire) {
            let Ok(from) = self.state.lock().map(|state| state.queue.scanned()) else {
                return;
            };
            let Ok(page) = self
                .ledger
                .query(&AuditFilter::default().after_offset(from).with_limit(PAGE))
            else {
                return;
            };
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            // 这一页是在锁外读的：这期间有请求自己往前读过，这一页就作废，从新位置重读。
            if state.queue.scanned() != from {
                continue;
            }
            for record in &page {
                state.queue.absorb(TARGET, record);
            }
            if page.len() < PAGE {
                return;
            }
        }
    }

    /// 队列已经读到的账本偏移；只给用例看进度。
    #[cfg(test)]
    pub(crate) fn scanned_offset(&self) -> u64 {
        self.state.lock().map_or(0, |state| state.queue.scanned())
    }
}
