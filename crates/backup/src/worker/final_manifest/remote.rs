use super::{FinalManifestDraft, FinalManifestReceipt};
use crate::worker::restic::ResticError;
use studiolo_core::checkpoint::InstantMillis;

/// 只读查询的实际结果；没有匹配项必须来自成功的完整查询，不能把网络错误映射为缺失。
pub enum FinalManifestLookup {
    /// 唯一记录的完整字节已读回，并与持久 draft 一致。
    Verified(Box<FinalManifestReceipt>),
    /// 查询完成时尚无记录；期限内仍保留未知，不能据此重传。
    Absent {
        /// 实际完整查询结束时读取的 UTC；不能由请求方指定。
        observed_at: InstantMillis,
    },
}

/// 此能力只读取非正文最终记录；本地传输源过期后仍可独立使用。
pub trait FinalManifestReader {
    /// 严格按原作业及确定 draft 查询；多个候选、字节不符、错误均返回 Err。
    fn read_final(&self, draft: &FinalManifestDraft) -> Result<FinalManifestLookup, ResticError>;
}

/// 提交能力须持有本次真实数据核验证据；不能仅相信调用方传入的 DTO。
pub trait FinalManifestWriter: FinalManifestReader {
    /// 最多执行一次提交并读回。Err 可能发生在远端写入之后，调用方必须持久保留未知。
    fn commit_final(&self, draft: &FinalManifestDraft)
    -> Result<FinalManifestReceipt, ResticError>;
}
