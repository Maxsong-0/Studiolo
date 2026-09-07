//! 服务共享状态：白名单只读根、当前代次、私有库、审计账本与降级窗口。

use std::sync::{Arc, Mutex, RwLock};

use studiolo_audit_ledger::{AuditEvent, AuditLedger, SqliteAuditLedger};
use studiolo_core::VaultPath;
use studiolo_supply::{AgentStore, ScanLimits, Snapshot, SupplyRoots, WorkbenchReadModel};

/// 一次完整扫描得到的不可变代次。
pub struct Generation {
    /// 代次号（从 1 起单调递增）。
    pub number: u64,
    /// 白名单快照。
    pub snapshot: Snapshot,
    /// 搜索/导航读模型。
    pub model: WorkbenchReadModel,
    /// 生成时间（RFC 3339）。
    pub generated_at: String,
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

/// 服务共享状态。
pub struct AppState {
    /// 白名单只读根。
    pub roots: SupplyRoots,
    /// 白名单前缀（配置顺序）。
    pub allow: Vec<VaultPath>,
    /// agent 私有库。
    pub store: AgentStore,
    /// 权威审计账本。
    pub ledger: SqliteAuditLedger,
    /// 当前代次。
    pub current: RwLock<Arc<Generation>>,
    /// 当前 outage（审计不可用时）。
    pub outage: Mutex<Option<Outage>>,
    /// 扫描预算。
    pub limits: ScanLimits,
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
        snapshot,
        model,
        generated_at: now(),
    })
}

impl AppState {
    /// 重扫并替换当前代次。
    pub fn rescan(&self) -> Result<u64, String> {
        let next = self.current().number + 1;
        let generation = Arc::new(build_generation(&self.roots, self.limits, next)?);
        *self
            .current
            .write()
            .map_err(|_| "代次锁不可用".to_owned())? = generation;
        Ok(next)
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
