//! 通过公开 Broker Interface 验证写者互斥、持久状态和失败关闭。

#![cfg(unix)]

#[path = "writer/history.rs"]
mod history;
#[path = "writer/legacy.rs"]
mod legacy;
#[path = "writer/limits.rs"]
mod limits;
#[path = "writer/locks.rs"]
mod locks;
#[path = "writer/opening.rs"]
mod opening;
#[path = "writer/receipts.rs"]
mod receipts;
#[path = "writer/replacement.rs"]
mod replacement;
#[path = "writer/storage.rs"]
mod storage;
#[path = "writer/support.rs"]
mod support;
