//! 完整校验的不可变副本；只持有 Broker 只读能力，不直接访问宿主文件系统。

mod protocol;
mod verify;

use std::path::Path;

use studiolo_core::VaultPath;
use studiolo_vault_broker::ReadOnlyVault;

use crate::Snapshot;
pub use protocol::Receipt;
use protocol::State;

/// 固定原因，不含源文件内容、宿主路径或底层错误。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReplicaError {
    /// 控制文件、时间、根或清单不合法。
    #[error("replica_invalid: {0}")]
    Invalid(&'static str),
    /// 已发布内容与清单不一致。
    #[error("replica_drift: {0}")]
    Drift(&'static str),
    /// 发布指针在验证期间改变；有界重试仍未获得一致代次。
    #[error("replica_superseded: 发布持续变化，请重试")]
    Superseded,
}

/// 与已验证内存快照绑定的新鲜度；重扫时间不覆盖来源时间。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Freshness {
    /// 本批来源回执。
    pub receipt: Receipt,
    /// 最近一次同步尝试的 Unix 毫秒时间。
    pub last_attempt_at_ms: i64,
    /// 最近一次尝试失败原因；成功时为空。
    pub last_error: Option<String>,
    /// 超过此秒数显示 stale。
    pub max_age_secs: u64,
}

impl Freshness {
    /// 动态计算 age；状态展示与内容同一代次。
    pub fn describe(&self, now_ms: i64) -> Result<String, ReplicaError> {
        self.state().validate(now_ms)?;
        let observed = self
            .receipt
            .source_observed_at_ms
            .min(self.receipt.received_at_ms);
        let age_ms = now_ms.saturating_sub(observed).max(0);
        let stale =
            age_ms > i64::try_from(self.max_age_secs.saturating_mul(1000)).unwrap_or(i64::MAX);
        let status = if self.last_error.is_some() {
            "sync_failed"
        } else if stale {
            "stale"
        } else {
            "ready"
        };
        Ok(format!(
            "副本状态 {status}；stale={stale}；generation={}；source_digest={}；source_observed_at_ms={}；received_at_ms={}；age_secs={}；max_age_secs={}{}",
            self.receipt.generation,
            self.receipt.source_digest,
            self.receipt.source_observed_at_ms,
            self.receipt.received_at_ms,
            age_ms / 1000,
            self.max_age_secs,
            self.last_error
                .as_ref()
                .map_or(String::new(), |error| format!(
                    "；last_error={error}（读取最后验证副本）"
                ))
        ))
    }

    fn state(&self) -> State {
        State {
            schema_version: 1,
            status: if self.last_error.is_some() {
                "failed"
            } else {
                "ready"
            }
            .into(),
            last_attempt_at_ms: self.last_attempt_at_ms,
            last_error: self.last_error.clone(),
            receipt: Some(self.receipt.clone()),
        }
    }
}

/// 一个完整校验的代次，不含任何仍指向可变文件的句柄。
pub struct VerifiedReplica {
    /// 供给用的完整内存快照。
    pub snapshot: Snapshot,
    /// 与快照绑定的来源状态。
    pub freshness: Freshness,
}

/// 在启动时固定的私有副本根；白名单必须与每次发布清单完全一致。
pub struct ReplicaRoot {
    root: ReadOnlyVault,
    allow: Vec<VaultPath>,
    max_age_secs: u64,
}

impl ReplicaRoot {
    /// 打开现有私有根；任何目录都不会被创建。
    pub fn open(root: &Path, allow: &[String], max_age_secs: u64) -> Result<Self, ReplicaError> {
        if !(60..=86_400).contains(&max_age_secs) {
            return Err(ReplicaError::Invalid("max_age_range"));
        }
        Ok(Self {
            root: ReadOnlyVault::open_private(root)
                .map_err(|_| ReplicaError::Invalid("unsafe_root"))?,
            allow: protocol::allowlist(allow)?,
            max_age_secs,
        })
    }

    /// 有界校验全部清单、文件字节与可见目录；前后控制文件必须逐字节一致。
    /// 发布并发改变时最多重试三次；旧回执不得回退或改绑另一内容。
    pub fn verify(
        &self,
        now_ms: i64,
        previous: Option<&Freshness>,
    ) -> Result<VerifiedReplica, ReplicaError> {
        for _ in 0..3 {
            let before = self.control()?;
            let state: State =
                serde_json::from_slice(&before).map_err(|_| ReplicaError::Invalid("state_json"))?;
            let receipt = state.validate(now_ms)?;
            if let Some(previous) = previous {
                let old = &previous.receipt;
                if receipt.source_observed_at_ms < old.source_observed_at_ms
                    || receipt.received_at_ms < old.received_at_ms
                    || state.last_attempt_at_ms < previous.last_attempt_at_ms
                    || (receipt.generation == old.generation
                        && receipt.source_digest != old.source_digest)
                {
                    return Err(ReplicaError::Invalid("receipt_regression"));
                }
            }
            let verified = verify::generation(&self.root, &self.allow, receipt);
            let after = self.control()?;
            if before != after {
                continue;
            }
            return Ok(VerifiedReplica {
                snapshot: verified?,
                freshness: Freshness {
                    receipt: receipt.clone(),
                    last_attempt_at_ms: state.last_attempt_at_ms,
                    last_error: state.last_error,
                    max_age_secs: self.max_age_secs,
                },
            });
        }
        Err(ReplicaError::Superseded)
    }

    fn control(&self) -> Result<Vec<u8>, ReplicaError> {
        let path = VaultPath::parse("control/state.json").expect("fixed path");
        self.root
            .read_file(&path, 4096)
            .map(|note| note.content)
            .map_err(|_| ReplicaError::Invalid("unreadable_state"))
    }
}
