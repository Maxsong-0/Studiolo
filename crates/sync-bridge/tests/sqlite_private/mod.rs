//! 真实临时 SQLite 契约草稿；未运行。内存审计用例不代表生产身份验收。

mod corruption;
mod deadlines;
mod executor_port;
mod executor_port_failure;
mod executor_port_recovery;
mod lifecycle;
mod limits;
mod opening;
mod retention;
mod scale;
mod storage_contract;
mod support;

mod lease_release;

mod cancellation;
