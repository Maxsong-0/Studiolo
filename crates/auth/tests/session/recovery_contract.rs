//! 恢复码公开 Interface 契约；真实 SQLite 与虚构秘密，不替代上线秘密装配。

#[path = "recovery_clock_guard.rs"]
mod clock_guard;
#[path = "recovery_concurrency.rs"]
mod concurrency;
#[path = "recovery_durable.rs"]
mod durable;
#[path = "recovery_durable_support.rs"]
mod durable_support;
#[path = "factor_runtime.rs"]
mod factor_runtime;
#[path = "factor_state.rs"]
mod factor_state;
#[path = "factor_state_pending.rs"]
mod factor_state_pending;
#[path = "factor_state_recovery.rs"]
mod factor_state_recovery;
#[path = "recovery_failures.rs"]
mod failures;
#[path = "recovery_flows.rs"]
mod flows;
#[path = "http_ready_recovered.rs"]
mod http_ready;
#[path = "recovery_integrity.rs"]
mod integrity;
#[path = "recovery_issuance.rs"]
mod issuance;
#[path = "recovery_limits.rs"]
mod limits;
#[path = "recovery_longevity.rs"]
mod longevity;
#[path = "recovery_migration.rs"]
mod migration;
#[path = "recovery_pending.rs"]
mod pending;
#[path = "recovery_privacy.rs"]
mod privacy;
#[path = "recovery_receipts.rs"]
mod receipts;
#[path = "recovery_support.rs"]
mod support;
#[path = "recovery_time.rs"]
mod time;

use super::*;
use support::*;
