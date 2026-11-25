//! 打包的判据由可信组合根给：最近一次成功封存盖到哪、现在几点。写者不认识备份作业，也不读账本。
use super::format::Kind;

/// 满这么久才退役（用户 2026-11-24 定的 7 天）。
pub(super) const RETIRE_AGE_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// 这一轮打包的判据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveCutoff {
    /// 最近一次成功封存盖到的审计账本 offset（含）。两份回执的 offset 都不超过它，
    /// 散件就一定整组在那次封存里：封存要求待投递为空、每条请求两份回执都在。
    pub sealed_offset: u64,
    /// 现在的 Unix 毫秒。
    pub now_ms: i64,
}
impl ArchiveCutoff {
    /// 两份回执都在封存水位之内。
    pub(super) fn sealed(self, offsets: [u64; 2]) -> bool {
        offsets.iter().all(|offset| *offset <= self.sealed_offset)
    }
    /// 满了年限。没有时间、时钟比请求还早，都不算满。
    pub(super) fn aged(self, at_ms: Option<i64>) -> bool {
        at_ms
            .and_then(|at| self.now_ms.checked_sub(at))
            .is_some_and(|age| age >= RETIRE_AGE_MS)
    }
}

/// 一轮打包走到哪了；从头开始用 [`Default`]。一轮里看过、不合格的不再看第二遍。
#[derive(Clone, Debug, Default)]
pub struct ArchiveCursor {
    /// 看到哪一类、哪一个请求号；`None` 是还没开始。
    pub(super) after: Option<(Kind, String)>,
    pub(super) done: bool,
}

/// 一步的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveStep {
    /// 这一步打了几包。
    pub packed: usize,
    /// 这一步看了几个请求。
    pub examined: usize,
    /// 这一轮看完了。
    pub done: bool,
}
