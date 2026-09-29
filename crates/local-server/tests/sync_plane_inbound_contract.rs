//! 入站冻结源的真实持久化契约：临时 stage、唯一 canonical 写者与真实 Broker。
//! 镜像源使用文件读取夹具，不启动特权 launcher；不代表生产端到端验收。

#[path = "sync_plane_inbound/fixture.rs"]
mod fixture;
#[path = "sync_plane_inbound/replay.rs"]
mod replay;
#[path = "sync_plane_inbound/stage.rs"]
mod stage;
