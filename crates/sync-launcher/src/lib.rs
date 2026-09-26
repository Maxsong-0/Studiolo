//! 可信 launcher（ADR-025 D2）：`serve` 是 root 系统服务，经私有 Unix socket 只接受 owner
//! uid 的封闭请求，用固定绝对路径 `/usr/bin/systemd-run` 起 transient service，把 payload
//! 挡在放行 FIFO 后，核对隔离与 (dev,ino)，再回报 cgroup 事实；`payload` 是 unit 内的垫片：
//! 在最终 mount 视图内 stat 四个对象报回 launcher，阻塞读放行 FIFO，收到一字节后
//! `env_clear` 只加批准变量再 `exec` 封闭 argv。
//!
//! 线上契约唯一母本在 `studiolo_sync_transport::linux::protocol`；本 crate 只做两侧的
//! 状态机与 OS 调用。真实 IO 只许在 `serve.rs`（socket 与目录）、`payload.rs`（垫片的
//! stdio、stat、exec）、`unit.rs`（systemd-run／systemctl）、`unit/reads.rs`（cgroupfs／proc／
//! session 文件只读）、`unit/session.rs`（session 目录与控制 FIFO）与 `config.rs` 的两处固定
//! 读法（守卫按精确记号登记）；`facts.rs`、`handler.rs`、`protocol.rs`、`session.rs` 保持纯逻辑。
//!
//! 垫片拿不到 `LISTEN_FDS` 的裸 fd（工作区 `forbid(unsafe_code)`，`from_raw_fd` 不可用），
//! 0600 root 的固定配置对 payload 身份也不可读，所以 launcher 经 stdin 上的控制 FIFO 把
//! 启动事实（固定 pins、镜像根、配置根、操作）交给垫片，垫片把报告帧写在 stdout 文件头部；
//! 放行是同一 FIFO 上的一个字节，关门就是丢掉写端。

pub mod config;
pub mod facts;
pub mod handler;
pub mod payload;
pub mod protocol;
pub mod serve;
pub mod session;
pub mod unit;
