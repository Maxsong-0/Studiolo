//! 新根恢复里身份库这一步（T-212）：旧类型 token 全部吊销，交回全部实例标识给策略库记实例吊销。
//! 恢复编排在本地服务里，那边不能直接拿身份库句柄或调写身份的方法（架构守卫），所以入口放在这里。
use super::{AgentStore, SupplyError, checkpoint::restored_watermark, unavailable};
use std::path::Path;
use studiolo_core::checkpoint::OwnerWatermark;

/// 身份库这一步的结果；不带 token、摘要和会话信息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoredIdentities {
    /// 这次新吊销的类型数（之前已经吊销的不算）。
    pub types_revoked: usize,
    /// 已登记的全部类型数；做完之后它们全部是吊销状态。
    pub types_total: usize,
    /// 全部已登记实例的标识（按标识排序），留给策略库逐个记实例吊销。
    pub instances: Vec<String>,
}

/// 新根恢复第一次做身份库这一步之前的核对（只读）：只打开封存里放出来的已有库（不建、不迁移），按封存时
/// 同一套规则重算水位，和清单里的 `expected` 逐项相等才算过。核过了编排才把「开始」交出去、再调
/// [`invalidate_restored_identities`]；核不过什么都不动，恢复记录里也没有「开始」，接手时照旧先核。
pub fn verify_restored_identities(
    private_directory: &Path,
    expected: &OwnerWatermark,
) -> Result<(), SupplyError> {
    let store = AgentStore::open_existing(private_directory)?;
    if restored_watermark(&store)? != *expected {
        return Err(SupplyError::CorruptStore);
    }
    Ok(())
}

/// 新根恢复：只打开封存里放出来的已有库（不建、不迁移），吊销全部类型（旧 token 一律失效），
/// 核「没有一个类型还有效」，交回全部实例标识。幂等：再做一遍什么都不变，接手上一次时照样调。
/// 第一次做之前先过 [`verify_restored_identities`]。
pub fn invalidate_restored_identities(
    private_directory: &Path,
    now: &str,
) -> Result<RestoredIdentities, SupplyError> {
    let store = AgentStore::open_existing(private_directory)?;
    let types = store.types()?;
    let mut types_revoked = 0;
    for (name, _, _) in &types {
        if store.revoke_type(name, now)? {
            types_revoked += 1;
        }
    }
    if store.types()?.iter().any(|(_, _, revoked)| !revoked) {
        return Err(SupplyError::CorruptStore);
    }
    let instances = {
        let connection = store.lock()?;
        let mut statement = connection
            .prepare("SELECT id FROM agent_instance ORDER BY id")
            .map_err(unavailable)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(unavailable)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)?
    };
    Ok(RestoredIdentities {
        types_revoked,
        types_total: types.len(),
        instances,
    })
}
