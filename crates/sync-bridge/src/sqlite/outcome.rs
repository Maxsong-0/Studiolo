//! 可信执行适配器的闭集回执；结构校验不能证明来源身份。
use crate::Digest;
use serde::{Deserialize, Serialize};

/// 原键终态：后端结果或 BridgeState 原子取消；普通 JSON、退出码及相同 hash 均非证据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectReceipt {
    /// 原动作已经持久化的后端幂等键。
    pub external_key: String,
    /// 原输入及动作位置摘要。
    pub request_digest: Digest,
    /// 可信适配器验证过的后端根身份。
    pub backend: Backend,
    /// 已提交或明确无效果的终态。
    pub outcome: EffectOutcome,
}

/// Broker pair 身份由适配器验证；Mirror 不代表远端上传成功。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Backend {
    /// Canonical 的真实 Broker 执行结果。
    Broker {
        /// 原绑定的 canonical 根标识。
        canonical_root: Digest,
    },
    /// 专用镜像的受控安装结果。
    Mirror {
        /// 原绑定的镜像根标识。
        mirror_root: Digest,
    },
}

/// 原键持久终态，尚未核验或超时不能构造成功分支。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectOutcome {
    /// 后端确认原请求已经持久提交。
    Committed {
        /// 与原动作精确相同的 VaultPath 文本。
        path: String,
        /// 创建为 None，其余必须等于原目标 revision。
        previous_revision: Option<Digest>,
        /// 删除为 None，其余为实际新 revision。
        new_revision: Option<Digest>,
        /// 非删除必须匹配冻结源的内容摘要。
        content_hash: Option<Digest>,
        /// 非删除必须匹配冻结源的字节数。
        bytes: Option<u64>,
        /// 适配器核验过的真实持久回执标识。
        receipt_id: String,
    },
    /// 同原键明确无安装效果；NotFound、超时与同内容不构成此证据。
    NoEffect {
        /// 与原动作精确相同的路径。
        path: String,
        /// 与原动作相同的目标 CAS revision。
        expected_revision: Option<Digest>,
        /// 后端已确认的无效果原因。
        reason: NoEffectReason,
        /// 后端持久拒绝票据，或 BridgeState 从未发出动作的持久取消票据。
        terminal_id: String,
    },
}

/// 明确无安装效果的闭集原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoEffectReason {
    /// 原 CAS 条件未满足，未安装。
    CasRejected,
    /// 后端持久确认安装前取消，或 BridgeState 原子确认从未发出后消费预约。
    CancelledBeforeInstall,
}

pub use Backend as ReceiptBackend;
pub use EffectOutcome as ReceiptOutcome;
