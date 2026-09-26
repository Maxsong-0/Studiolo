//! 镜像视图、外 stage、冲突副本命名与恢复协议的契约：全部在 `/tmp` 下的临时目录里跑，
//! 不启动传输、不打开 Bridge 库、不碰真实 Vault。
#[path = "sync_plane_mirror/fixture.rs"]
mod fixture;
#[path = "sync_plane_mirror/naming.rs"]
mod naming;
#[path = "sync_plane_mirror/observe.rs"]
mod observe;
#[path = "sync_plane_mirror/recovery.rs"]
mod recovery;
#[path = "sync_plane_mirror/stage.rs"]
mod stage;
