//! 传输审批只读契约；只创建本次 /tmp 合成目录，不安装 Headless、不启动进程、不联网。

#[path = "sync_transport/behaviour.rs"]
mod behaviour;
#[path = "sync_transport/failure.rs"]
mod failure;
#[path = "sync_transport/fields.rs"]
mod fields;
#[path = "sync_transport/fixture.rs"]
mod fixture;
#[path = "sync_transport/recovery.rs"]
mod recovery;
