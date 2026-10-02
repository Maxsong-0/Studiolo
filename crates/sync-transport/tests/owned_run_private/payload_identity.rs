//! 生产形状回归：固定 launcher 与逐次 payload 是两个不同的 systemd 单元。

use super::support::*;
use crate::owned_run::*;

#[test]
fn the_payload_identity_is_bound_to_the_persisted_run_not_the_launcher_service() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let before = fixture.slot.state().active.expect("准备记录存在");
    session.start(&prepared).expect("真实形状的载荷身份应通过");
    let after = fixture.slot.state().active.expect("执行记录存在");
    let container = after.container.expect("固定载荷身份");
    assert_eq!(
        container.unit_name,
        format!(
            "studiolo-sync-payload-{}-{}.service",
            before.sequence,
            &before.run_id.to_string()[..8]
        )
    );
    assert_ne!(container.unit_name, SERVICE_UNIT);
    assert_eq!(after.phase, RunPhase::ExecutionIssued);
    assert_eq!(session.backend.releases, 1);
}

#[test]
fn launcher_or_foreign_run_unit_names_never_open_the_release_barrier() {
    for variant in 0..3 {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        let prepared = fixture.prepared(&mut session);
        let record = fixture.slot.state().active.expect("准备记录存在");
        let mut foreign = record.run_id.as_bytes();
        foreign[0] ^= 0xff;
        session.backend.unit_name_override = Some(match variant {
            0 => SERVICE_UNIT.to_owned(),
            1 => format!(
                "studiolo-sync-payload-{}-{}.service",
                record.sequence + 1,
                &record.run_id.to_string()[..8]
            ),
            _ => format!(
                "studiolo-sync-payload-{}-{}.service",
                record.sequence,
                &RunId::from_bytes(foreign).to_string()[..8]
            ),
        });
        assert_eq!(
            session.start(&prepared).err(),
            Some(TransportError::ExitUnconfirmed)
        );
        assert_eq!(session.backend.launches, 1);
        assert_eq!(session.backend.releases, 0);
        let failed = fixture.slot.state().active.expect("保留失败现场");
        assert_eq!(failed.phase, RunPhase::LaunchIssued);
        assert!(failed.container.is_none());
        let mut reopened = fixture.session();
        let observed = reopened.recover(record.run_id).expect("仅观察旧运行");
        assert_eq!(observed.quiescence, Quiescence::Unconfirmed);
        assert_eq!(observed.exit, ExitObservation::NotObserved);
        assert_eq!(
            reopened.prepare_once(2, RunOperation::RunOnce, 3).err(),
            Some(TransportError::RunActive)
        );
        assert_eq!(reopened.backend.launches, 0);
        assert_eq!(reopened.backend.releases, 0);
    }
}
