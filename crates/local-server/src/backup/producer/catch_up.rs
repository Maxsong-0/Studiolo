//! 人侧起来后自己把账本读到末尾，不靠外面的请求一页一页推（BUG-042）。
//!
//! 队列的进度只在内存里：人侧每次启动都从账本第一条读起。以前只有 worker、定时器、健康页的请求推它
//! 往前读，一条请求最多读 `SCAN_BUDGET`，账本有 70 万条时头十几分钟一条请求都答不上来。现在监听一起来
//! 就在后台线程里把它读完（`service.rs`），请求那条路一个字不变：还没读完时请求照旧自己接着读。
use super::{BackupProducer, TARGET};
use std::sync::atomic::{AtomicBool, Ordering};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

/// 后台追账本一页读多少条，比请求那条路的一页（`PAGE`，512 条）大得多。
///
/// 每次查询有一笔固定成本（核段链、读 head、走读打开段），页越小、查询次数越多，付得越多。
/// 定这个值时账本每次查询都从创世重算整条段链，108 上 512 条一页追 70 万条推算约 31 秒；T-220 之后每次查询
/// 只核链尾，这笔成本不再随账本涨，两种页差得不多了（108，T-219 合入 T-220 后：70 万条 512 条一页 8.62 秒、
/// 8192 条一页 7.68 秒）。大页仍省掉大部分查询，留着。
/// 代价是一页读得久一些：账本的锁一次多占一会儿，关停也最多多等这么久。
/// 请求那条路不跟着改：它拿着状态锁读，页小一点别的请求等得短。
const CATCH_UP_PAGE: usize = 8192;

impl BackupProducer {
    /// 读到账本末尾或被叫停为止；每读一页之前看一次停止标志，关停最多等手上这一页读完。
    pub fn catch_up(&self, stop: &AtomicBool) {
        self.catch_up_by(stop, CATCH_UP_PAGE);
    }

    /// `catch_up` 的本体，页大小由调用方给：用例拿小页来走「读满一页接着读下一页」。
    ///
    /// 账本在状态锁之外读，拿锁只做内存里的记账：请求和健康页等这把锁，等不到它头上。
    /// 读账本出错就收手，留给之后的请求照旧自己读；收手写一行固定类别的日志，
    /// 不然请求又要一页页从头推、现场却看不出为什么（BUG-042 的老样子）。
    pub(crate) fn catch_up_by(&self, stop: &AtomicBool, size: usize) {
        while !stop.load(Ordering::Acquire) {
            let Ok(from) = self.state.lock().map(|state| state.queue.scanned()) else {
                return abandoned("状态锁不可用");
            };
            let Ok(page) = self
                .ledger
                .query(&AuditFilter::default().after_offset(from).with_limit(size))
            else {
                return abandoned("读账本失败");
            };
            let Ok(mut state) = self.state.lock() else {
                return abandoned("状态锁不可用");
            };
            // 这一页是在锁外读的：这期间有请求自己往前读过，这一页就作废，从新位置重读。
            if state.queue.scanned() != from {
                continue;
            }
            for record in &page {
                state.queue.absorb(TARGET, record);
            }
            if page.len() < size {
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

/// 后台追账本没读到末尾就收手：固定类别，不含路径和内容。
fn abandoned(reason: &str) {
    eprintln!("备份队列后台追账本中止：{reason}");
}
