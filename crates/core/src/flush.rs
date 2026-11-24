//! 拷大文件时每写一段就落一次盘（BUG-054）。
//!
//! 写完才落一次盘的话，没落盘的页一路攒到整个文件那么大。服务跑在有内存上限的 cgroup 里，没落盘的页回收不了：
//! 169 上账本 693 MB、上限 512 MiB，2026-11-24 07:10Z 内核 OOM 杀掉人侧，网站停了 31 分钟。
//! 按段落盘之后，没落盘的量和文件大小脱钩；只多几次 `fdatasync`，写完本来就要落盘。

/// 每写这么多字节落一次盘。
const EVERY_BYTES: u64 = 64 << 20;

/// 写了多少字节还没落盘；到了就该落一次。这里只数数，落盘由写文件的那一方给。
pub struct Flush {
    every: u64,
    since: u64,
}
impl Default for Flush {
    fn default() -> Self {
        Self::every(EVERY_BYTES)
    }
}
impl Flush {
    /// 每隔多少字节落一次（正式的写入用默认值，这里给用例用）；0 当 1，每写一次都落。
    pub fn every(bytes: u64) -> Self {
        Self {
            every: bytes.max(1),
            since: 0,
        }
    }
    /// 又写了 `bytes` 字节：攒够一段就落盘一次、从头攒。落盘失败原样交回，由写的那一方作废这次写入。
    pub fn written<E>(
        &mut self,
        bytes: u64,
        flush: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), E> {
        self.since = self.since.saturating_add(bytes);
        if self.since < self.every {
            return Ok(());
        }
        self.since = 0;
        flush()
    }
}

#[cfg(test)]
mod tests {
    use super::{EVERY_BYTES, Flush};

    /// 每次写 `step` 字节、一共 `steps` 次，数落了几次盘。
    fn flushes(mut flush: Flush, step: u64, steps: u64) -> u64 {
        let mut count = 0;
        for _ in 0..steps {
            flush
                .written(step, || -> Result<(), ()> {
                    count += 1;
                    Ok(())
                })
                .unwrap();
        }
        count
    }

    #[test]
    fn a_segment_is_64_mib() {
        assert_eq!(EVERY_BYTES, 64 * 1024 * 1024);
        // 4 KiB 的页、一步 64 页：256 步一次；没满一段一次都不落。
        let step = 64 * 4096;
        assert_eq!(flushes(Flush::default(), step, 255), 0);
        assert_eq!(flushes(Flush::default(), step, 256), 1);
        assert_eq!(flushes(Flush::default(), step, 1024), 4);
    }

    #[test]
    fn nothing_is_left_unflushed_for_more_than_one_segment() {
        // 693 MB 的账本（169，2026-11-24）：一步 256 KiB，落 10 次盘。
        let step = 64 * 4096;
        let steps = 693_268_480_u64.div_ceil(step);
        assert_eq!(flushes(Flush::default(), step, steps), 10);
    }

    #[test]
    fn a_write_larger_than_a_segment_flushes_every_time() {
        assert_eq!(flushes(Flush::default(), u64::MAX, 3), 3);
        assert_eq!(flushes(Flush::every(0), 1, 3), 3);
        // 什么都没写，不落。
        assert_eq!(flushes(Flush::every(0), 0, 3), 0);
    }

    #[test]
    fn a_flush_that_fails_is_reported() {
        let mut flush = Flush::every(1);
        assert_eq!(flush.written(1, || Err("落不了盘")), Err("落不了盘"));
    }
}
