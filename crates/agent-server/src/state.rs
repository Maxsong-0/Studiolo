//! 服务共享状态：白名单只读根、当前代次、私有库、审计账本与降级窗口。

use crate::index::{IndexRuntime, IndexStatus, ModelSource, Persistence};

use std::sync::{Arc, Mutex, RwLock};

use studiolo_audit_ledger::{AuditEvent, AuditLedger, SqliteAuditLedger};
use studiolo_core::VaultPath;
use studiolo_policy::PolicyAgentStore;
use studiolo_supply::{
    AgentStore, ScanLimits, Snapshot, SupplyRoots, WorkbenchReadModel,
    replica::{Freshness, ReplicaRoot},
};

/// 一次完整扫描得到的不可变代次。
pub struct Generation {
    /// 代次号（从 1 起单调递增）。
    pub number: u64,
    /// 白名单快照。
    pub snapshot: Snapshot,
    /// 搜索/导航读模型。
    pub model: Arc<WorkbenchReadModel>,
    /// 本次派生索引的复用与持久化结果。
    pub index_status: IndexStatus,
    /// 生成时间（RFC 3339）。
    pub generated_at: String,
    /// 副本来源状态；直接 Vault 模式没有可声明的同步回执。
    pub freshness: Option<Freshness>,
}

/// 审计不可用窗口。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outage {
    /// outage id（ULID）。
    pub id: String,
}

/// 账本已接受的记录位置与时间。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditAck {
    /// 账本偏移。
    pub offset: u64,
    /// 记录时间（Unix 毫秒）。
    pub recorded_at_ms: i64,
    /// 记录时间（RFC 3339）。
    pub recorded_at: String,
}

/// 追加审计失败时返回，携带当前 outage id；调用方必须拒绝返回任何内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditUnavailable {
    /// 当前 outage id。
    pub outage_id: String,
}

/// 供给源；两种模式都只持有只读能力。
pub enum SupplySource {
    /// 直接读取本地白名单目录。
    Direct(SupplyRoots),
    /// 读取完整验证的不可变服务器副本。
    Replica(ReplicaRoot),
}

impl SupplySource {
    /// 每次先完整读取并验证来源；缓存只能接收成功后的快照。
    pub(crate) fn scan(
        &self,
        limits: ScanLimits,
        previous: Option<&Freshness>,
    ) -> Result<(Snapshot, Option<Freshness>), String> {
        match self {
            Self::Direct(roots) => roots
                .scan(limits)
                .map(|snapshot| (snapshot, None))
                .map_err(|error| format!("白名单扫描失败：{error}")),
            Self::Replica(root) => root
                .verify(now_ms(), previous)
                .map(|batch| (batch.snapshot, Some(batch.freshness)))
                .map_err(|error| error.to_string()),
        }
    }
}

/// 服务共享状态。
pub struct AppState {
    /// 白名单只读根。
    pub source: SupplySource,
    /// 序列化重扫，避免慢旧批次覆盖较新的已验证代次。
    pub scan_lock: Mutex<()>,
    /// 白名单前缀（配置顺序）。
    pub allow: Vec<VaultPath>,
    /// agent 私有库。
    pub store: AgentStore,
    /// 策略平面（接入方视角）；`None` ＝ 未接入（M3 兼容模式：白名单只来自 `--allow`，无能力集、无吊销表）。
    pub policy: Option<PolicyAgentStore>,
    /// 私有 Unix 黑板连接；不持 canonical 文件能力。
    pub board: Option<crate::board_client::BoardClient>,
    /// 权威审计账本。
    pub ledger: SqliteAuditLedger,
    /// 当前代次。
    pub current: RwLock<Arc<Generation>>,
    /// 当前 outage（审计不可用时）。
    pub outage: Mutex<Option<Outage>>,
    /// 扫描预算。
    pub limits: ScanLimits,
    /// 已验证源的派生缓存。
    pub(crate) index: IndexRuntime,
    /// 关停状态：两个外壳据此拒绝新请求，MCP 传输据此排空后关会话，后台重扫据此停下。
    pub(crate) lifecycle: Arc<crate::shutdown::Lifecycle>,
}

/// 当前时间（RFC 3339，UTC）。
pub fn now() -> String {
    jiff::Timestamp::now().to_string()
}

/// 当前时间（Unix 毫秒）。
pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// 扫描白名单并构建一个新代次；扫描或建模失败返回错误。
pub fn build_generation(
    roots: &SupplyRoots,
    limits: ScanLimits,
    number: u64,
) -> Result<Generation, String> {
    let snapshot = roots
        .scan(limits)
        .map_err(|error| format!("白名单扫描失败：{error}"))?;
    let model = WorkbenchReadModel::build(&snapshot.documents())
        .map_err(|error| format!("读模型构建失败：{error}"))?;
    Ok(Generation {
        number,
        index_status: IndexStatus {
            source: ModelSource::Rebuilt,
            parsed: snapshot.notes.len(),
            reused: 0,
            persistence: Persistence::Disabled,
        },
        snapshot,
        model: Arc::new(model),
        generated_at: now(),
        freshness: None,
    })
}

/// 从一个完整校验的来源回执构建读模型。
pub fn replica_generation(
    root: &ReplicaRoot,
    previous: Option<&Freshness>,
    number: u64,
) -> Result<Generation, String> {
    let replica = root
        .verify(now_ms(), previous)
        .map_err(|error| error.to_string())?;
    let model = WorkbenchReadModel::build(&replica.snapshot.documents())
        .map_err(|_| "副本读模型构建失败".to_owned())?;
    Ok(Generation {
        number,
        index_status: IndexStatus {
            source: ModelSource::Rebuilt,
            parsed: replica.snapshot.notes.len(),
            reused: 0,
            persistence: Persistence::Disabled,
        },
        snapshot: replica.snapshot,
        model: Arc::new(model),
        generated_at: now(),
        freshness: Some(replica.freshness),
    })
}

impl AppState {
    /// 重扫并替换当前代次。
    pub fn rescan(&self) -> Result<u64, String> {
        let _scan = self
            .scan_lock
            .lock()
            .map_err(|_| "重扫锁不可用".to_owned())?;
        let previous = self.current();
        let next = previous
            .number
            .checked_add(1)
            .ok_or_else(|| "代次溢出".to_owned())?;
        let (snapshot, freshness) = self.source.scan(self.limits, previous.freshness.as_ref())?;
        let (model, index_status) = self.index.build(&snapshot, Some(&previous))?;
        let generation = Arc::new(Generation {
            number: next,
            snapshot,
            model,
            index_status,
            generated_at: now(),
            freshness,
        });
        *self
            .current
            .write()
            .map_err(|_| "代次锁不可用".to_owned())? = generation;
        Ok(next)
    }

    /// 请求供给时取完整验证的同一代次；副本全量校验在阻塞线程完成。
    pub async fn verified(self: &Arc<Self>) -> Result<Arc<Generation>, String> {
        if matches!(&self.source, SupplySource::Replica(_)) {
            let state = self.clone();
            return tokio::task::spawn_blocking(move || {
                state.rescan()?;
                Ok(state.current())
            })
            .await
            .map_err(|_| "副本验证任务失败".to_owned())?;
        }
        Ok(self.current())
    }

    /// 当前代次（克隆 Arc）。
    pub fn current(&self) -> Arc<Generation> {
        match self.current.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// 追加权威审计；失败即进入/延续降级窗口并返回 outage id，成功则结束窗口。
    pub fn audit(&self, event: AuditEvent) -> Result<AuditAck, AuditUnavailable> {
        match self.ledger.append(event) {
            Ok(record) => {
                let ended = self.outage.lock().ok().and_then(|mut guard| guard.take());
                if let Some(outage) = ended {
                    let _ = self.store.record_outage_ended(&outage.id, &now());
                }
                Ok(AuditAck {
                    offset: record.offset(),
                    recorded_at_ms: record.recorded_at().as_millisecond(),
                    recorded_at: record.recorded_at().to_string(),
                })
            }
            Err(_) => Err(self.enter_outage("audit_unavailable")),
        }
    }

    /// 进入或延续降级窗口；每次拒绝计数加一（有界补记落私有库，失败不影响拒绝本身）。
    pub fn enter_outage(&self, reason: &str) -> AuditUnavailable {
        let mut guard = match self.outage.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let outage = guard.get_or_insert_with(|| {
            let id = ulid::Ulid::generate().to_string();
            let _ = self.store.record_outage_started(&id, &now(), reason);
            Outage { id }
        });
        let _ = self.store.record_outage_denial(&outage.id);
        AuditUnavailable {
            outage_id: outage.id.clone(),
        }
    }

    /// 当前是否处于降级窗口。
    pub fn degraded(&self) -> Option<Outage> {
        self.outage.lock().ok().and_then(|guard| guard.clone())
    }
}
