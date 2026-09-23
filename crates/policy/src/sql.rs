//! 两个句柄共用的 SQL 片段：当前版本、校验上下文、effective 哈希、吊销状态、行映射。
//!
//! 所有函数都在调用方给定的事务内执行，不自己开连接。

mod current;
mod desired;
mod meta;
mod revocation;
mod version;

// 按表分文件；两个句柄与建库（database/schema.rs）经 `sql::` 取用下列函数。
pub(crate) use current::{capability_set, context, current_entries, current_records};
pub(crate) use desired::{desired, desired_list, insert_desired, update_desired};
pub(crate) use meta::{effective_hash, empty_effective_hash};
pub(crate) use revocation::{revocations, revoked};
pub(crate) use version::{insert_version, version_record, versions};
