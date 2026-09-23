//! 可信 launcher（ADR-025 D2）：`serve` 是 root 系统服务，经私有 Unix socket 只接受 owner
//! uid 的封闭请求，用固定绝对路径 `/usr/bin/systemd-run` 起 transient service，把 payload
//! 挡在放行 FIFO 后，核对隔离与 (dev,ino)，再回报 cgroup 事实；`payload` 是 unit 内的垫片：
//! 在最终 mount 视图内 stat 四个对象报回 launcher，阻塞读放行 FIFO，收到一字节后
//! `env_clear` 只加批准变量再 `exec` 封闭 argv。
//!
//! 线上契约唯一母本在 `studiolo_sync_transport::linux::protocol`；本 crate 只做两侧的
//! 状态机与 OS 调用。真实 IO 只许在 `serve.rs`／`payload.rs`／`unit.rs`（W2 新建，守卫按
//! 固定文件登记）；`config.rs`、`protocol.rs`、`session.rs` 保持纯逻辑。

pub mod config;
pub mod protocol;
pub mod session;
