//! 公开会话 Interface 契约；仅私有测试能取得固定虚构 Access 验证结果。
#![cfg(unix)]

mod budget;
mod concurrency;
mod credential_v2;
mod durable;
mod failures;
mod http_issuance;
mod http_readiness;
mod http_ready_durable;
mod http_ready_interleavings;
mod http_ready_support;
mod http_retention;
mod http_session;
mod http_support;
mod integrity;
mod intent_receipt;
mod interleavings;
mod lifecycle;
mod longevity;
mod migration;
mod migration_fixture;
mod open_existing;
mod recovery_contract;
mod retention;
mod retention_concurrency;
mod retention_durable;
mod retention_faults;
mod retention_integrity;
mod retention_parallel;
mod retention_pending;
mod retention_receipts;
mod signed_identity;
mod support;
mod totp_contract;
mod write_failures;

use super::*;
use support::*;
