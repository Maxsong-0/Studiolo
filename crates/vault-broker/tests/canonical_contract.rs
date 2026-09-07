//! T-022: the canonical read handle has no creation or mutation authority.
#![cfg(unix)]

#[path = "canonical/identity.rs"]
mod identity;
#[cfg(target_os = "linux")]
#[path = "canonical/linux.rs"]
mod linux;
#[path = "canonical/reads.rs"]
mod reads;
#[path = "canonical/support.rs"]
mod support;
