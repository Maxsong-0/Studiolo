//! 健康与告警审批的只读契约；不打开文件、不读环境、不联网。

#[path = "alerting/behaviour.rs"]
mod behaviour;
#[path = "alerting/failure.rs"]
mod failure;
#[path = "alerting/fixture.rs"]
mod fixture;
#[path = "alerting/recovery.rs"]
mod recovery;
