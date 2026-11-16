//! 身份列表的策略投影：同一事务中的哈希、类型能力与各级吊销。
use super::{GLOBAL_TARGET, PolicyStore, RevocationLevel, RevocationRecord};
use crate::{Authorization, Denial, PolicyError, sql};
use std::collections::BTreeMap;
/// 某类型的当前策略，不含能力正文。
pub struct TypePolicyState {
    /// 当前能力集版本（即使正被吊销，也保留其版本号）。
    pub capability_version: Option<u64>,
    /// 类型策略吊销。
    pub revocation: Option<RevocationRecord>,
}
/// 某实例的直接复核结果；祖先连带由持有身份链的调用方计算。
pub struct InstancePolicyState {
    /// 全局 → 类型 → 实例 → 能力集的复核结果。
    pub denial: Option<Denial>,
    /// 该实例自身的吊销记录。
    pub revocation: Option<RevocationRecord>,
}
/// 列表所需所有策略状态来自同一读事务。
pub struct AgentPolicySnapshot {
    /// 当前生效策略摘要。
    pub effective_hash: String,
    /// 全局吊销是否生效。
    pub global_revoked: bool,
    /// 类型名到公开策略。
    pub types: BTreeMap<String, TypePolicyState>,
    /// 实例号到直接复核结果，含列表窗口外祖先。
    pub instances: BTreeMap<String, InstancePolicyState>,
}
impl PolicyStore {
    /// 与接入方相同的一次授权复核。
    pub fn authorization(
        &self,
        agent_type: &str,
        instance_id: Option<&str>,
    ) -> Result<Authorization, PolicyError> {
        self.db
            .read(|tx| sql::authorization(tx, agent_type, instance_id))
    }
    /// `instances` 是实例号到类型名；调用方须包含返回行的全部祖先。
    pub fn agent_snapshot(
        &self,
        types: &[String],
        instances: &BTreeMap<String, String>,
    ) -> Result<AgentPolicySnapshot, PolicyError> {
        self.db.read(|tx| {
            let mut result = AgentPolicySnapshot {
                effective_hash: sql::effective_hash(tx)?,
                global_revoked: sql::revoked(tx, RevocationLevel::Global, GLOBAL_TARGET)?,
                types: BTreeMap::new(),
                instances: BTreeMap::new(),
            };
            for name in types {
                result.types.insert(
                    name.clone(),
                    TypePolicyState {
                        capability_version: sql::capability_set(tx, name)?
                            .map(|(_, meta)| meta.version),
                        revocation: sql::active_revocation(tx, RevocationLevel::Type, name)?,
                    },
                );
            }
            for (id, name) in instances {
                result.instances.insert(
                    id.clone(),
                    InstancePolicyState {
                        denial: sql::authorization(tx, name, Some(id))?.denial,
                        revocation: sql::active_revocation(tx, RevocationLevel::Instance, id)?,
                    },
                );
            }
            Ok(result)
        })
    }
}
