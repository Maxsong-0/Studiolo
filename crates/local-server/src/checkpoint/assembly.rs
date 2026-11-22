//! 启动保留原实例，备份请求才固定封存能力；身份来源从实际平面推导，不能另填 absent。
use super::*;

/// 可信启动层传入正在服务的原实例；本对象本身不暂停业务、不创建库、不签发备份票据。
#[derive(Clone)]
pub struct CheckpointInputs {
    /// 唯一 canonical 工作台。
    pub workbench: Arc<CanonicalWorkbench>,
    /// 实际 Auth 管理器及原主人绑定。
    pub sessions: Arc<SessionManager>,
    /// 实际的两步验证运行时；没启用因子模式就是 None。冻结之前靠它（没有就靠 `sessions`）补记登录库的待发审计。
    pub factors: Option<Arc<FactorRuntime>>,
    /// 实际人侧权威账本与完整操作门禁。
    pub audit: Arc<SqliteAuditLedger>,
    /// 实际黑板，None 仅表示没有装配。
    pub board: Option<Arc<BoardPlane>>,
    /// 实际双向同步平面。
    pub bridge: Option<Arc<SyncPlane>>,
    /// 实际策略发布平面及共享许可池。
    pub policy: Option<Arc<PolicyPlane>>,
}
impl CheckpointInputs {
    /// 从保留的原实例固定一次作业的源；失败不回退到缺省 owner 或新初始化的空库。
    pub fn bind(&self) -> Result<CheckpointOwners> {
        CheckpointOwners::new(self.clone())
    }
    pub(super) fn agents(&self) -> Result<Option<AgentCheckpointAssembly>> {
        let board = self
            .board
            .as_ref()
            .map(|plane| plane.checkpoint_agents())
            .transpose()
            .map_err(|_| unavailable(Owner::AgentStore))?;
        let policy = self
            .policy
            .as_ref()
            .map(|plane| plane.checkpoint_agents())
            .transpose()
            .map_err(|_| unavailable(Owner::AgentStore))?
            .flatten();
        match (board, policy) {
            (Some(board), Some(policy)) => {
                if !board
                    .same_origin(&policy)
                    .map_err(|_| unavailable(Owner::AgentStore))?
                {
                    return Err(unavailable(Owner::AgentStore));
                }
                Ok(Some(board))
            }
            (Some(source), None) | (None, Some(source)) => Ok(Some(source)),
            (None, None) => Ok(None),
        }
    }
}
