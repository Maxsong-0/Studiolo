//! 打包的判据由可信组合根给：最近一次成功封存盖到哪、现在几点。写者不认识备份作业，也不读账本。
use super::format::{HEADER_BYTES, Part, SEGMENT_BYTES, SEGMENT_PACKS};

/// 满这么久才退役（用户 2026-11-24 定的 7 天）。
pub(super) const RETIRE_AGE_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// 这一轮打包的判据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveCutoff {
    /// 最近一次成功封存盖到的审计账本 offset（含）。两份回执的 offset 都不超过它，
    /// 散件就一定整组在那次封存里：封存要求待投递为空、每条请求两份回执都在。
    pub sealed_offset: u64,
    /// 同一次成功封存的内容截止时刻（Unix 毫秒）。没有审计主体的写入没有回执、也就没有账本 offset，按它判（T-249）：
    /// 封存先等人侧空下来、冻结之后才取截止时刻，请求时间早于它的那一组一定整个在那次封存里。没有就是 `None`，这类不打。
    pub sealed_content_ms: Option<i64>,
    /// 现在的 Unix 毫秒。
    pub now_ms: i64,
}
impl ArchiveCutoff {
    /// 两份回执都在封存水位之内。
    pub(super) fn sealed(self, offsets: [u64; 2]) -> bool {
        offsets.iter().all(|offset| *offset <= self.sealed_offset)
    }
    /// 没有回执的请求：请求时间早于最近一次成功封存的内容截止时刻。没有时间、没有截止时刻都不算。
    pub(super) fn sealed_before(self, at_ms: Option<i64>) -> bool {
        matches!((at_ms, self.sealed_content_ms), (Some(at), Some(cutoff)) if at < cutoff)
    }
    /// 满了年限。没有时间、时钟比请求还早，都不算满。
    pub(super) fn aged(self, at_ms: Option<i64>) -> bool {
        at_ms
            .and_then(|at| self.now_ms.checked_sub(at))
            .is_some_and(|age| age >= RETIRE_AGE_MS)
    }
}

/// 凑多少包才写成一段。段数有上限、段又不能合并：合格的陆续到期时要是有几包写几包，段数会先用光。
/// 凑不够的留散件，不影响合格、不影响读写。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveBatch {
    /// 平时：一轮看完凑不够半段就不写。
    Usual,
    /// 有一处散件到了八成、都没到 95%：小段也写，但只有几包的仍然不写——腾不出多少，段却一样占一个。
    Pressed,
    /// 剩多少写多少，一包也写。有一处散件到了 95%：再不腾就要拒绝写入了，多占段也写；
    /// 或者操作者停服手工打：一条命令最多多出一个小段。
    All,
}
impl ArchiveBatch {
    /// 一轮看完之后，剩下的至少几包才写成一段。装满的段不看它。
    pub(super) fn least(self) -> usize {
        match self {
            Self::Usual => SEGMENT_PACKS / 2,
            Self::Pressed => SEGMENT_PACKS / 8,
            Self::All => 1,
        }
    }
}

/// 一轮打包走到哪了；从头开始用 [`Default`]。一轮里看过、不合格的不再看第二遍。
/// 合格的先只记请求号，凑够一段再写；写的那一步在锁内逐个重核、重读，不信上一步看到的。
#[derive(Clone, Debug, Default)]
pub struct ArchiveCursor {
    /// 看到哪一个目录（按它的第一条认：正文描述符、文件操作请求）、哪一个请求号；`None` 是还没开始。
    pub(super) after: Option<(Part, String)>,
    /// 这一轮开头量的：散件到了 95%，已在封存之内、不满 7 天的也打（用户 2026-11-27 定）。`None` 是还没量。
    pub(super) urgent: Option<bool>,
    /// 合格、还没写成段的。
    held: Vec<(Part, String)>,
    /// 上面那些一共几字节。
    held_bytes: usize,
    /// 装满了一段：包数到顶，或者再装一个字节就超了。
    full: bool,
    /// 这一轮全看过了。
    pub(super) scanned: bool,
}
impl ArchiveCursor {
    /// 记下一个合格的；这一段装不下就不记，交回假。
    pub(super) fn hold(&mut self, group: Part, id: &str, bytes: usize) -> bool {
        if HEADER_BYTES + self.held_bytes + bytes > SEGMENT_BYTES {
            self.full = true;
            return false;
        }
        self.held.push((group, id.to_owned()));
        self.held_bytes += bytes;
        self.full = self.held.len() >= SEGMENT_PACKS;
        true
    }
    /// 该写成一段了。
    pub(super) fn ready(&self, batch: ArchiveBatch) -> bool {
        self.full || self.scanned && self.held.len() >= batch.least()
    }
    /// 装满了，不再往下看。
    pub(super) fn full(&self) -> bool {
        self.full
    }
    /// 取走记下的，准备写。
    pub(super) fn take(&mut self) -> Vec<(Part, String)> {
        self.held_bytes = 0;
        self.full = false;
        std::mem::take(&mut self.held)
    }
}

/// 一步的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveStep {
    /// 这一步打了几包。
    pub packed: usize,
    /// 这一步往前看了几个请求；写段的那一步不往前看。
    pub examined: usize,
    /// 这一轮看完了，该写的也写完了。
    pub done: bool,
}
