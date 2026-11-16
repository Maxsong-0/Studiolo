//! 人侧只读身份视图：同一受控开库入口，SQLite 只读标志与公开类型共同禁止写入。
use super::{AgentStore, CreatePolicy, Instance, SupplyError, children, schema, unavailable};
use rusqlite::{Connection, TransactionBehavior};
use std::{collections::BTreeMap, path::Path};

/// 不含凭据摘要的类型状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeIdentity {
    /// 类型名。
    pub name: String,
    /// 最近签发时间。
    pub issued_at: String,
    /// 身份凭据吊销时间。
    pub revoked_at: Option<String>,
}
/// 不含会话摘要的实例状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceIdentity {
    /// 已登记的实例。
    pub instance: Instance,
    /// 父实例；格式 0 恒为空。
    pub parent: Option<String>,
    /// 最近一次授权调用；登记时以登记时间初始化，不表示在线。
    pub last_seen_at: String,
}
/// 一个身份库读事务内的列表与祖先；不把窗口外祖先当成不存在。
pub struct IdentitySnapshot {
    /// 已登记类型。
    pub types: Vec<TypeIdentity>,
    /// 新到旧、有界的实例。
    pub instances: Vec<InstanceIdentity>,
    /// 每个返回实例从自身到根的链。
    pub lineages: BTreeMap<String, Vec<Instance>>,
    /// 是否已显式升级格式 1。
    pub children_enabled: bool,
}
/// 只能读已有身份库；无初始化、迁移、认证摘要或任何写方法。
pub struct AgentIdentityView {
    store: AgentStore,
}
impl AgentIdentityView {
    /// 总协调器只接收这份身份库与其固定附属 Audit 成对能力，不允许另传账本路径。
    pub fn checkpoint_assembly(&self) -> Result<super::AgentCheckpointAssembly, SupplyError> {
        super::checkpoint::assemble(&self.store)
    }

    /// 从正在使用的只读身份源取得固定封存能力；不增加身份创建、认证或迁移入口。
    pub fn checkpoint_source(&self) -> Result<super::AgentCheckpointSource, SupplyError> {
        super::checkpoint::source(&self.store)
    }
    /// 从同一已批准身份根下的固定 audit 子目录打开只读账本；不创建缺失库。
    pub fn read_audit(
        &self,
    ) -> Result<crate::read_audit::AgentReadAudit, studiolo_audit_ledger::AuditError> {
        self.store
            .storage
            .check()
            .map_err(|_| studiolo_audit_ledger::AuditError::UnsafeStorage)?;
        let root = self
            .store
            .storage
            .database
            .parent()
            .ok_or(studiolo_audit_ledger::AuditError::UnsafeStorage)?;
        let view = crate::read_audit::AgentReadAudit::open(&root.join("audit"))?;
        self.store
            .storage
            .check()
            .map_err(|_| studiolo_audit_ledger::AuditError::UnsafeStorage)?;
        Ok(view)
    }
    /// 验证已有库，拒绝缺失、替换、损坏、未知格式与非 WAL 库。
    pub fn open_existing(directory: &Path) -> Result<Self, SupplyError> {
        Ok(Self {
            store: AgentStore::open_mode(directory, CreatePolicy::ExistingOnly, true)?,
        })
    }
    /// 人侧组合根使用：身份目录不能被内容/静态资源服务，也不能与其它私有根重叠。
    pub fn open_existing_isolated(
        directory: &Path,
        isolated: &[&Path],
    ) -> Result<Self, SupplyError> {
        crate::storage::ensure_separate(directory, isolated)?;
        Self::open_existing(directory)
    }
    /// 类型的公开状态。
    pub fn types(&self) -> Result<Vec<TypeIdentity>, SupplyError> {
        types(&*self.store.lock()?)
    }
    /// 当前类型是否有效。
    pub fn type_is_active(&self, name: &str) -> Result<bool, SupplyError> {
        self.children_enabled()?;
        self.store.type_is_active(name)
    }
    /// 自身到根的有界链。
    pub fn instance_lineage(&self, id: &str) -> Result<Vec<Instance>, SupplyError> {
        let mut db = self.store.lock()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(unavailable)?;
        version(&tx)?;
        let value = children::lineage(&tx, id)?;
        self.store.storage.check()?;
        tx.commit().map_err(unavailable)?;
        Ok(value)
    }
    /// 是否支持子实例。
    pub fn children_enabled(&self) -> Result<bool, SupplyError> {
        Ok(version(&*self.store.lock()?)? == 1)
    }
    /// 最多 10000 个实例，含父号。
    pub fn instances_with_parents(
        &self,
        limit: usize,
    ) -> Result<Vec<InstanceIdentity>, SupplyError> {
        Ok(self.snapshot(limit)?.instances)
    }
    /// 同一只读事务读取类型、实例、祖先与格式，防止读取途中注册/吊销造成混合身份状态。
    pub fn snapshot(&self, limit: usize) -> Result<IdentitySnapshot, SupplyError> {
        let mut db = self.store.lock()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(unavailable)?;
        let children_enabled = version(&tx)? == 1;
        let types = types(&tx)?;
        let instances = instances(&tx, limit, children_enabled)?;
        let lineages = instances
            .iter()
            .map(|row| {
                Ok((
                    row.instance.id.clone(),
                    children::lineage(&tx, &row.instance.id)?,
                ))
            })
            .collect::<Result<_, SupplyError>>()?;
        self.store.storage.check()?;
        tx.commit().map_err(unavailable)?;
        Ok(IdentitySnapshot {
            types,
            instances,
            lineages,
            children_enabled,
        })
    }
}
fn version(db: &Connection) -> Result<i64, SupplyError> {
    match schema::version(db)? {
        n @ (0 | 1) => Ok(n),
        _ => Err(SupplyError::CorruptStore),
    }
}
fn types(db: &Connection) -> Result<Vec<TypeIdentity>, SupplyError> {
    version(db)?;
    let mut statement = db
        .prepare("SELECT name, issued_at, revoked_at FROM agent_type ORDER BY name")
        .map_err(unavailable)?;
    statement
        .query_map([], |row| {
            Ok(TypeIdentity {
                name: row.get(0)?,
                issued_at: row.get(1)?,
                revoked_at: row.get(2)?,
            })
        })
        .map_err(unavailable)?
        .collect::<Result<_, _>>()
        .map_err(unavailable)
}
fn instances(
    db: &Connection,
    limit: usize,
    children: bool,
) -> Result<Vec<InstanceIdentity>, SupplyError> {
    let sql = if children {
        "SELECT i.id,i.type_name,i.registered_at,i.last_seen_at,p.parent_id FROM agent_instance i LEFT JOIN instance_parent p ON p.child_id=i.id ORDER BY i.registered_at DESC,i.id DESC LIMIT ?1"
    } else {
        "SELECT id,type_name,registered_at,last_seen_at,NULL FROM agent_instance ORDER BY registered_at DESC,id DESC LIMIT ?1"
    };
    let mut statement = db.prepare(sql).map_err(unavailable)?;
    statement
        .query_map([limit.clamp(1, 10000) as i64], |row| {
            Ok(InstanceIdentity {
                instance: Instance {
                    id: row.get(0)?,
                    agent_type: row.get(1)?,
                    registered_at: row.get(2)?,
                },
                last_seen_at: row.get(3)?,
                parent: row.get(4)?,
            })
        })
        .map_err(unavailable)?
        .collect::<Result<_, _>>()
        .map_err(unavailable)
}
#[cfg(test)]
#[path = "../../tests/identity_view/private.rs"]
mod tests;
