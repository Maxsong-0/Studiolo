//! 平台契约：受控隔离前置未满足时如实报缺口，绝不用 mock 把平台标记为可用。

use super::support::*;
use crate::linux::{ContainmentGap, verify_backend};
use crate::owned_run::*;

#[test]
fn no_platform_can_currently_produce_a_verified_linux_backend() {
    let approval = FakeApproval::new();
    let gap = verify_backend(&approval).expect_err("当前没有任何平台能通过前置检查");

    if cfg!(target_os = "linux") {
        // Linux 上缺的是管理员预置的可信 launcher/service 特权，不是「平台不对」。
        assert_eq!(gap, ContainmentGap::TrustedLauncherNotProvisioned);
    } else {
        assert_eq!(gap, ContainmentGap::UnsupportedPlatform);
    }
}

#[test]
fn an_unacceptable_service_unit_is_rejected_before_any_platform_probe() {
    let approval = FakeApproval::new();
    approval.backend.borrow_mut().service_unit = "studiolo-sync.timer".to_owned();
    assert_eq!(
        verify_backend(&approval).expect_err("非 service unit 一律拒绝"),
        ContainmentGap::ProtocolViolation
    );
}

#[test]
fn a_stale_approval_never_reaches_the_platform_probe() {
    let approval = FakeApproval::new();
    *approval.fresh.borrow_mut() = false;
    assert_eq!(
        verify_backend(&approval).expect_err("配置已漂移"),
        ContainmentGap::IdentityMismatch
    );
}

#[test]
fn every_gap_before_the_start_request_is_reported_as_unsupported_containment() {
    for gap in [
        ContainmentGap::UnsupportedPlatform,
        ContainmentGap::TrustedLauncherNotProvisioned,
        ContainmentGap::CgroupV2Missing,
        ContainmentGap::DelegationMissing,
        ContainmentGap::ControllerMissing,
        ContainmentGap::KillCapabilityMissing,
        ContainmentGap::NamespaceIsolationMissing,
        ContainmentGap::DedicatedIdentityMissing,
        ContainmentGap::DiskQuotaMissing,
        ContainmentGap::BudgetUnsupported,
    ] {
        assert_eq!(gap.before_start(), TransportError::UnsupportedContainment);
        // 启动请求已发出之后一律保持未知，绝不回退成「从未执行」。
        assert_eq!(gap.after_start(), TransportError::ExitUnconfirmed);
    }
}

#[test]
fn the_pinned_runtime_only_accepts_the_exact_published_release() {
    let approval = FakeApproval::new();
    assert_eq!(
        PinnedRuntime::from_approval(&approval)
            .expect("固定运行时可取")
            .pins()
            .version,
        PINNED_RUNTIME_VERSION
    );

    struct Drifted(FakeApproval, RuntimePins);
    impl ApprovedTransport for Drifted {
        fn revalidate(&self) -> Result<()> {
            self.0.revalidate()
        }
        fn binding(&self) -> Result<TransportBindingFacts> {
            self.0.binding()
        }
        fn roots(&self) -> RootPins {
            self.0.roots()
        }
        fn runtime(&self) -> RuntimePins {
            self.1.clone()
        }
        fn backend(&self) -> BackendDescriptor {
            self.0.backend()
        }
        fn budget(&self) -> RunBudget {
            self.0.budget()
        }
    }

    for pins in [
        RuntimePins {
            version: "0.0.15".to_owned(),
            ..FakeApproval::new().runtime
        },
        RuntimePins {
            node_path: std::path::PathBuf::from("node"),
            ..FakeApproval::new().runtime
        },
        RuntimePins {
            cli_path: FakeApproval::new().runtime.node_path.clone(),
            ..FakeApproval::new().runtime
        },
    ] {
        let drifted = Drifted(FakeApproval::new(), pins);
        assert_eq!(
            PinnedRuntime::from_approval(&drifted).err(),
            Some(TransportError::RuntimeChanged)
        );
    }
}
