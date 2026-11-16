//! 真实 staging 的绑定纯核验；不冒充 root 配置/systemd 凭据/双 UID 的运行验收。
use crate::seal_fixture::*;
use studiolo_backup::worker::{
    audit::AdmissionFact, final_manifest::FinalManifestDraft, recovery_point::BackupTargetKind,
    restic::SnapshotRef, state::RemoteVerification,
};
#[test]
fn admitted_binding_matches_sealed_source_and_original_expired_noncontent_draft() {
    let fixture = Fixture::new();
    let sealed = fixture.seal("admitted-source");
    let key = job("admitted-source", 1);
    let fact = AdmissionFact::from_sealed(
        key.clone(),
        BackupTargetKind::DedicatedAppendOnlyTarget,
        &sealed,
    )
    .unwrap();
    assert!(fact.verify_sealed(&sealed).is_ok());
    assert!(fact.check_live().is_err(), "夹具源已经过期，不能交付正文");
    let draft = FinalManifestDraft::new(
        key,
        &sealed,
        SnapshotRef::parse("ab".repeat(32)).unwrap(),
        RemoteVerification {
            objects_complete: true,
            checksum_verified: true,
            paired_checkpoint_reference: true,
            verified_at: ms(1100),
        },
    )
    .unwrap();
    assert!(
        fact.verify_draft(&draft).is_ok(),
        "过期不禁止原最终记录的只读核对"
    );
    assert!(fact.verify_sealed(&fixture.seal("other-source")).is_err());
    for (field, value) in [
        (
            "source",
            serde_json::json!({"checkpoint_id":"admitted-source","seal_sha256":"cd".repeat(32)}),
        ),
        ("expires_at", serde_json::json!(10000)),
        ("content_cutoff", serde_json::json!(901)),
        ("audit_offset", serde_json::json!(2)),
        ("audit_hash", serde_json::json!("cd".repeat(32))),
        (
            "key",
            serde_json::json!({"sequence":2,"checkpoint_id":"admitted-source"}),
        ),
    ] {
        let mut value_fact = serde_json::to_value(&fact).unwrap();
        value_fact[field] = value;
        let changed: AdmissionFact = serde_json::from_value(value_fact).unwrap();
        assert!(changed.verify_draft(&draft).is_err(), "{field}");
        if field != "key" {
            assert!(changed.verify_sealed(&sealed).is_err(), "{field}");
        }
    }
}
