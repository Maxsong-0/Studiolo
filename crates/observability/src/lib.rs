//! 健康聚合、磁盘与同步配额、带外告警与死人开关。
//! 告警 transport 是 trait，真实凭据只经环境变量注入，绝不落仓库。
//! 持久状态 IO 收敛在 `store::storage`；实际推送 IO 只在 `alert::push` 的发送适配器中。

pub mod alert;
pub mod health;
pub mod quota;
pub mod store;
