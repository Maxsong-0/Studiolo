//! 健康聚合、磁盘与同步配额、带外告警与死人开关。
//! 告警 transport 是 trait，真实凭据只经环境变量注入，绝不落仓库。
//! 真实 IO 全部收敛在 `store::storage`，其余模块保持纯逻辑。

pub mod alert;
pub mod health;
pub mod quota;
pub mod store;
