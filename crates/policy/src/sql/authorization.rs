//! 人侧列表和接入方实时复核共用授权顺序，全部读取调用方事务。
use crate::{
    Authorization, CapabilityGrant, Denial, PolicyError, sql,
    store::{GLOBAL_TARGET, RevocationLevel},
};
use rusqlite::Transaction;
pub(crate) fn authorization(
    tx: &Transaction<'_>,
    agent_type: &str,
    instance_id: Option<&str>,
) -> Result<Authorization, PolicyError> {
    let effective_hash = sql::effective_hash(tx)?;
    let denial = if sql::revoked(tx, RevocationLevel::Global, GLOBAL_TARGET)? {
        Some(Denial::GlobalRevoked)
    } else if sql::revoked(tx, RevocationLevel::Type, agent_type)? {
        Some(Denial::TypeRevoked)
    } else if let Some(instance) = instance_id
        && sql::revoked(tx, RevocationLevel::Instance, instance)?
    {
        Some(Denial::InstanceRevoked)
    } else {
        None
    };
    if let Some(denial) = denial {
        return Ok(Authorization {
            effective_hash,
            grant: None,
            denial: Some(denial),
        });
    }
    match sql::capability_set(tx, agent_type)? {
        Some((set, meta)) => Ok(Authorization {
            effective_hash,
            grant: Some(CapabilityGrant { set, meta }),
            denial: None,
        }),
        None => Ok(Authorization {
            effective_hash,
            grant: None,
            denial: Some(Denial::NoCapabilitySet),
        }),
    }
}
