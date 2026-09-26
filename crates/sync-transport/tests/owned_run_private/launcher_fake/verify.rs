//! `verify_backend_at` 对假 launcher 的探测契约；从 `launcher_fake.rs` 拆出只为压住行数。

use super::super::support::*;
use super::{FakeLauncher, Probe};
use crate::linux::protocol::Request;
use crate::linux::{ContainmentGap, verify_backend_at};

#[test]
fn probe_facts_from_a_launcher_produce_a_verified_backend_only_when_every_item_holds() {
    let launcher = FakeLauncher::start();
    let approval = FakeApproval::new();
    let backend = verify_backend_at(&approval, launcher.socket()).expect("全部前置实测成立");
    assert_eq!(backend.service_unit(), SERVICE_UNIT);
    assert_eq!(backend.payload_identity(), (PAYLOAD_UID, PAYLOAD_GID));
    assert_eq!(backend.socket(), launcher.socket());
    // 探测只发 probe，且每次 nonce 都不同。
    verify_backend_at(&approval, launcher.socket()).expect("可重复探测");
    let script = launcher.script();
    let nonces = script
        .requests
        .iter()
        .map(|request| match request {
            Request::Probe { nonce } => *nonce,
            other => panic!("探测不该发 {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(nonces.len(), 2);
    assert_ne!(nonces[0], nonces[1]);
}

#[test]
fn each_failed_probe_item_reports_its_own_gap_and_never_a_backend() {
    let launcher = FakeLauncher::start();
    let approval = FakeApproval::new();
    for (probe, gap) in [
        (Probe::WithoutCgroupV2, ContainmentGap::CgroupV2Missing),
        (Probe::WithoutLoopQuota, ContainmentGap::DiskQuotaMissing),
        (
            Probe::WithoutDedicatedIdentity,
            ContainmentGap::DedicatedIdentityMissing,
        ),
        (Probe::WrongIdentity, ContainmentGap::IdentityMismatch),
        (Probe::WrongVersion, ContainmentGap::ProtocolViolation),
        (Probe::WrongNonce, ContainmentGap::ProtocolViolation),
        (Probe::PeerRefused, ContainmentGap::PeerRejected),
    ] {
        launcher.script().probe = probe;
        assert_eq!(
            verify_backend_at(&approval, launcher.socket()).err(),
            Some(gap),
            "{probe:?}"
        );
    }
    launcher.script().probe = Probe::Ok;
    assert!(verify_backend_at(&approval, launcher.socket()).is_ok());
}

#[test]
fn a_stale_approval_or_an_unacceptable_unit_never_reaches_the_socket() {
    let launcher = FakeLauncher::start();
    let stale = FakeApproval::new();
    *stale.fresh.borrow_mut() = false;
    assert_eq!(
        verify_backend_at(&stale, launcher.socket()).err(),
        Some(ContainmentGap::IdentityMismatch)
    );
    let bad_unit = FakeApproval::new();
    bad_unit.backend.borrow_mut().service_unit = "studiolo-sync.timer".to_owned();
    assert_eq!(
        verify_backend_at(&bad_unit, launcher.socket()).err(),
        Some(ContainmentGap::ProtocolViolation)
    );
    assert!(launcher.script().requests.is_empty());
    // 没人监听的端点：如实报「未置备」，不是「平台不对」。
    let missing = launcher.directory().join("missing").join("launcher.sock");
    assert_eq!(
        verify_backend_at(&FakeApproval::new(), &missing).err(),
        Some(ContainmentGap::TrustedLauncherNotProvisioned)
    );
}
