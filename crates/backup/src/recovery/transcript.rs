//! 恢复 transcript：任何恢复写入之前先建立，带完整性保护且不含秘密。
//!
//! 完整性保护是逐条哈希链；秘密检查复用审计的 `SanitizedPayload`，
//! 凡是看起来像密码/令牌/密钥的字段名一律拒绝进入 transcript。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use studiolo_audit_ledger::SanitizedPayload;
use studiolo_core::checkpoint::CheckpointId;

/// transcript 中的恢复步骤。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStep {
    /// 恢复 kit 已在本机 TTY/localhost 打开。
    KitOpened,
    /// 已在同步与真实执行 Adapter 禁用下完成前置检查。
    PreRestoreChecksCompleted,
    /// 主人与密钥映射已恢复。
    OwnerAndKeyMappingRestored,
    /// 新目录物理重绑定的 Intent。
    PhysicalRebindIntent,
    /// 新目录物理重绑定的 Outcome。
    PhysicalRebindOutcome,
    /// 开始对一个 owner 做恢复后的失效（载荷写是哪个 owner、哪次恢复）。接手上一次时据此知道它的库已经动过，
    /// 不再按封存水位重核，只核失效之后该有的状态。
    OwnerRestoreIntent,
    /// Control 里拿着租约的 attempt（已认领、运行中）全部收回：没有未结束子任务的直接放弃，
    /// 有的转成取消中（工具能力收回，只等子任务结束）。
    ControlLeasesRevoked,
    /// 策略库全局吊销，只能由主人经 L3 解除。
    PolicyGloballyRevoked,
    /// AgentStore 全部类型 token 吊销，全部实例在策略库记实例吊销。
    AgentCredentialsInvalidated,
    /// 双向同步保持关闭：Bridge 的运行状态没有恢复，只能按引导流程从零重新打开。
    SyncLeftClosed,
    /// Auth 旧会话、L3 与恢复码失效。
    AuthInvalidated,
    /// 最终切换的 Intent。
    FinalCutoverIntent,
    /// 最终切换的 Outcome。
    FinalCutoverOutcome,
    /// 回滚或保持只读停服。
    RolledBackOrHalted,
}

/// transcript 的一条记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptEntry {
    /// 序号，从 1 开始。
    pub sequence: u64,
    /// 步骤。
    pub step: RecoveryStep,
    /// 已检查无秘密字段的载荷。
    pub payload: SanitizedPayload,
    /// 前一条的链哈希。
    pub previous_hash: String,
    /// 本条的链哈希。
    pub entry_hash: String,
}

/// 一份带完整性保护的恢复 transcript。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryTranscript {
    /// 本次恢复所基于的 checkpoint。
    pub checkpoint_id: CheckpointId,
    /// 逐条记录。
    pub entries: Vec<TranscriptEntry>,
}

/// 恢复记录逐行文本的第一行：只有 checkpoint 标识，之后每行一条记录，都是 JSON。命令行边做边写这种文本
/// （每追加一条就写一行），接手和最终切换时再整份读回来。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptHeader {
    /// 本次恢复所基于的 checkpoint。
    pub checkpoint_id: CheckpointId,
}

/// transcript 已建立的凭证；没有它就不允许发生任何恢复写入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteAuthorization {
    _private: (),
}

impl RecoveryTranscript {
    /// 建立一份新的 transcript；创世哈希绑定 checkpoint 标识。
    #[must_use]
    pub fn begin(checkpoint_id: CheckpointId) -> Self {
        Self {
            checkpoint_id,
            entries: Vec::new(),
        }
    }

    /// 当前链头。
    #[must_use]
    pub fn head(&self) -> String {
        self.entries.last().map_or_else(
            || genesis(&self.checkpoint_id),
            |entry| entry.entry_hash.clone(),
        )
    }

    /// 追加一条记录；含秘密字段的载荷一律拒绝。
    pub fn append(
        &mut self,
        step: RecoveryStep,
        payload: serde_json::Value,
    ) -> Result<&TranscriptEntry, TranscriptError> {
        let payload =
            SanitizedPayload::try_from(payload).map_err(|_| TranscriptError::SecretInTranscript)?;
        let previous_hash = self.head();
        let sequence = self.entries.len() as u64 + 1;
        let entry_hash = chain(&previous_hash, sequence, step, &payload)?;
        self.entries.push(TranscriptEntry {
            sequence,
            step,
            payload,
            previous_hash,
            entry_hash,
        });
        self.entries.last().ok_or(TranscriptError::BrokenChain)
    }

    /// 重新校验整条链。
    pub fn verify(&self) -> Result<(), TranscriptError> {
        let mut previous = genesis(&self.checkpoint_id);
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.sequence != index as u64 + 1 || entry.previous_hash != previous {
                return Err(TranscriptError::BrokenChain);
            }
            if chain(
                &entry.previous_hash,
                entry.sequence,
                entry.step,
                &entry.payload,
            )? != entry.entry_hash
            {
                return Err(TranscriptError::BrokenChain);
            }
            previous = entry.entry_hash.clone();
        }
        Ok(())
    }

    /// 逐行文本的第一行（[`TranscriptHeader`]）。
    pub fn header_line(&self) -> Result<String, TranscriptError> {
        serde_json::to_string(&TranscriptHeader {
            checkpoint_id: self.checkpoint_id.clone(),
        })
        .map_err(|_| TranscriptError::BrokenChain)
    }

    /// 一条记录在逐行文本里的那一行。
    pub fn entry_line(entry: &TranscriptEntry) -> Result<String, TranscriptError> {
        serde_json::to_string(entry).map_err(|_| TranscriptError::BrokenChain)
    }

    /// 从逐行文本整份读回：第一行是头，之后每行一条记录（空行跳过）。每条载荷照追加时一样查秘密字段，
    /// 整条链必须自洽；行不完整、多字段、少字段都拒绝。
    pub fn from_lines(text: &str) -> Result<Self, TranscriptError> {
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        let header: TranscriptHeader = lines
            .next()
            .and_then(|line| serde_json::from_str(line).ok())
            .ok_or(TranscriptError::BrokenChain)?;
        let entries = lines
            .map(serde_json::from_str::<TranscriptEntry>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| TranscriptError::BrokenChain)?;
        let transcript = Self {
            checkpoint_id: header.checkpoint_id,
            entries,
        };
        transcript.verify()?;
        Ok(transcript)
    }

    /// 取得恢复写入授权；必须已经记下 `KitOpened` 且整条链自洽。
    pub fn authorize_write(&self) -> Result<WriteAuthorization, TranscriptError> {
        if self.entries.is_empty() {
            return Err(TranscriptError::TranscriptNotEstablished);
        }
        if self.entries[0].step != RecoveryStep::KitOpened {
            return Err(TranscriptError::TranscriptNotEstablished);
        }
        self.verify()?;
        Ok(WriteAuthorization { _private: () })
    }
}

fn genesis(checkpoint_id: &CheckpointId) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"studiolo-recovery-transcript-v1\0");
    hasher.update(checkpoint_id.as_str().as_bytes());
    hex(&hasher.finalize())
}

fn chain(
    previous: &str,
    sequence: u64,
    step: RecoveryStep,
    payload: &SanitizedPayload,
) -> Result<String, TranscriptError> {
    let encoded =
        serde_json::to_vec(payload.as_value()).map_err(|_| TranscriptError::BrokenChain)?;
    let step = serde_json::to_vec(&step).map_err(|_| TranscriptError::BrokenChain)?;
    let mut hasher = Sha256::new();
    hasher.update(previous.as_bytes());
    hasher.update(b"\0");
    hasher.update(sequence.to_be_bytes());
    hasher.update(&step);
    hasher.update(b"\0");
    hasher.update(&encoded);
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        // 写入 String 不会失败；这里不静默吞掉任何真实错误。
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

/// transcript 的失败模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptError {
    /// 载荷里含有可能是秘密的字段。
    SecretInTranscript,
    /// 哈希链不自洽。
    BrokenChain,
    /// 尚未建立 transcript 就要求恢复写入。
    TranscriptNotEstablished,
}

impl std::fmt::Display for TranscriptError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::SecretInTranscript => "恢复 transcript 不得包含秘密字段",
            Self::BrokenChain => "恢复 transcript 的完整性链不自洽",
            Self::TranscriptNotEstablished => "任何恢复写入之前必须先建立 transcript",
        })
    }
}

impl std::error::Error for TranscriptError {}
