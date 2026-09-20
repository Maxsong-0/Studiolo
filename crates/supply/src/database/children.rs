//! 子实例关系只授予身份从属，不授予权限；读取时必须复核全部祖先。
use super::{AgentStore, Instance, SupplyError, identity::sha256_hex, schema, unavailable};
use rusqlite::{Connection, OptionalExtension as _, TransactionBehavior, params};

impl AgentStore {
    /// 显式将已验证的旧身份库升级为支持子实例的格式；不隐式升级 serve。
    pub fn upgrade_child_instances(&self) -> Result<(), SupplyError> {
        let mut db = self.lock()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        schema::upgrade_children(&tx)?;
        self.storage.check()?;
        tx.commit().map_err(unavailable)
    }

    /// 注册同类型的子实例，最大深度三；调用方已认证当前类型并复核父实例授权。
    pub fn register_child_instance(
        &self,
        agent_type: &str,
        parent_id: &str,
        session_id: &str,
        now: &str,
    ) -> Result<Instance, SupplyError> {
        let mut db = self.lock()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        if schema::version(&tx)? != 1 {
            return Err(SupplyError::IdentityUpgradeRequired);
        }
        let ancestors = lineage(&tx, parent_id)?;
        if ancestors.len() > 3 || ancestors.iter().any(|i| i.agent_type != agent_type) {
            return Err(SupplyError::InvalidParent);
        }
        let active: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_type WHERE name=?1 AND revoked_at IS NULL)",
                [agent_type],
                |r| r.get(0),
            )
            .map_err(unavailable)?;
        if !active {
            return Err(SupplyError::InvalidParent);
        }
        let instance = Instance {
            id: ulid::Ulid::generate().to_string(),
            agent_type: agent_type.to_owned(),
            registered_at: now.to_owned(),
        };
        tx.execute(
            "INSERT INTO agent_instance(id,type_name,session_digest,registered_at,last_seen_at) VALUES(?1,?2,?3,?4,?4)",
            params![instance.id, agent_type, sha256_hex(session_id), now],
        ).map_err(unavailable)?;
        tx.execute(
            "INSERT INTO instance_parent(child_id,parent_id) VALUES(?1,?2)",
            params![instance.id, parent_id],
        )
        .map_err(unavailable)?;
        self.storage.check()?;
        tx.commit().map_err(unavailable)?;
        Ok(instance)
    }

    /// 自身到根的有界身份链；缺失或损坏失败关闭，旧库返回仅自身。
    pub fn instance_lineage(&self, id: &str) -> Result<Vec<Instance>, SupplyError> {
        let db = self.lock()?;
        lineage(&db, id)
    }
}

fn lineage(db: &Connection, id: &str) -> Result<Vec<Instance>, SupplyError> {
    let children = schema::version(db)? == 1;
    let mut next = Some(id.to_owned());
    let mut result: Vec<Instance> = Vec::new();
    while let Some(current) = next {
        if result.len() >= 4 || result.iter().any(|i| i.id == current) {
            return Err(SupplyError::InvalidParent);
        }
        let instance = db
            .query_row(
                "SELECT id,type_name,registered_at FROM agent_instance WHERE id=?1",
                [&current],
                |r| {
                    Ok(Instance {
                        id: r.get(0)?,
                        agent_type: r.get(1)?,
                        registered_at: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(unavailable)?
            .ok_or(SupplyError::InvalidParent)?;
        next = if children {
            db.query_row(
                "SELECT parent_id FROM instance_parent WHERE child_id=?1",
                [&current],
                |r| r.get(0),
            )
            .optional()
            .map_err(unavailable)?
        } else {
            None
        };
        result.push(instance);
    }
    Ok(result)
}
