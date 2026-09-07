//! TOTP 公开 Interface 验收；辅助与分组文件仅在本测试树中注册。

#[path = "totp_concurrency.rs"]
mod concurrency;
#[path = "totp_durable.rs"]
mod durable;
#[path = "totp_enrollment_state.rs"]
mod enrollment_state;
#[path = "totp_failures.rs"]
mod failures;
#[path = "totp_flows.rs"]
mod flows;
#[path = "totp_integrity.rs"]
mod integrity;
#[path = "totp_limits.rs"]
mod limits;
#[path = "totp_longevity.rs"]
mod longevity;
#[path = "totp_migration.rs"]
mod migration;
#[path = "totp_privacy.rs"]
mod privacy;
#[path = "totp_receipt_guards.rs"]
mod receipt_guards;
#[path = "totp_sources.rs"]
mod sources;
#[path = "totp_support.rs"]
mod support;
#[path = "totp_time.rs"]
mod time;

use super::*;
use support::*;
