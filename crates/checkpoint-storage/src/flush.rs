//! 导出拷贝时每拷一段就把目标落一次盘（BUG-054）。
//!
//! 备份循环自己只在拷完之后落一次盘，没落盘的页一路攒到整个库那么大。服务跑在有内存上限的 cgroup 里，
//! 没落盘的页回收不了：169 上账本 693 MB、上限 512 MiB，2026-11-24 07:10Z 内核 OOM 杀掉人侧，网站停了 31 分钟。
//! 按段落盘之后，没落盘的量和库的大小脱钩；只多几次 `fdatasync`，拷完本来就要落盘。
use crate::Error;

/// 每拷这么多字节落一次盘。
const EVERY_BYTES: u64 = 64 << 20;

/// 攒了多少页还没落盘；到了就该落一次。
pub(crate) struct Flush {
    every_pages: u64,
    since: u64,
}
impl Flush {
    /// 按库的页大小算每隔多少页落一次；页比一段还大也至少一页一次。
    pub(crate) fn new(page_bytes: u64) -> Self {
        Self::every(EVERY_BYTES / page_bytes.max(1))
    }
    /// 用例用：直接给定每隔多少页。
    pub(crate) fn every(pages: u64) -> Self {
        Self {
            every_pages: pages.max(1),
            since: 0,
        }
    }
    /// 又拷了 `pages` 页：攒够一段就落盘一次、从头攒。落不了盘就是目标不可用，整份导出失败。
    pub(crate) fn copied(
        &mut self,
        pages: u64,
        flush: impl FnOnce() -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.since += pages;
        if self.since < self.every_pages {
            return Ok(());
        }
        self.since = 0;
        flush()
    }
}

#[cfg(test)]
mod tests {
    use super::{EVERY_BYTES, Flush};
    use crate::Error;

    /// 一步 64 页、一共 `steps` 步，数落了几次盘。
    fn flushes(mut flush: Flush, steps: u64) -> u64 {
        let mut count = 0;
        for _ in 0..steps {
            flush
                .copied(64, || {
                    count += 1;
                    Ok(())
                })
                .unwrap();
        }
        count
    }

    #[test]
    fn a_segment_is_64_mib_at_the_default_page_size() {
        assert_eq!(EVERY_BYTES, 64 * 1024 * 1024);
        // 4 KiB 的页：一段 16384 页，一步 64 页，256 步一次；没满一段一次都不落。
        assert_eq!(flushes(Flush::new(4096), 255), 0);
        assert_eq!(flushes(Flush::new(4096), 256), 1);
        assert_eq!(flushes(Flush::new(4096), 1024), 4);
        // 页大一倍，步数减半。
        assert_eq!(flushes(Flush::new(8192), 128), 1);
    }

    #[test]
    fn nothing_is_left_unflushed_for_more_than_one_segment() {
        // 693 MB 的账本（169，2026-11-24）：4 KiB 一页、一步 64 页，落 10 次盘。
        let pages = 693_268_480_u64.div_ceil(4096);
        assert_eq!(flushes(Flush::new(4096), pages.div_ceil(64)), 10);
    }

    #[test]
    fn an_enormous_page_still_flushes_every_step() {
        assert_eq!(flushes(Flush::new(u64::MAX), 3), 3);
        assert_eq!(flushes(Flush::new(0), 3), 0);
        assert_eq!(flushes(Flush::every(0), 3), 3);
    }

    #[test]
    fn a_flush_that_fails_is_reported() {
        let mut flush = Flush::every(1);
        assert_eq!(
            flush.copied(1, || Err(Error::Unavailable)),
            Err(Error::Unavailable)
        );
    }
}
