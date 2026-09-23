//! 水位之外的清单契约：固定事实（已 ACK 覆盖证明、包摘要、部署清单、秘密引用）
//! 与持久清单的 schema／JSON 往返。

use super::super::support::{DIGEST, manifest_inputs, ticket};
use super::build;
use studiolo_backup::checkpoint::manifest::{
    AuditCoverageProof, CheckpointManifest, ManifestError, SecretReference, SecretSource,
};
use studiolo_core::VaultUuid;
use tempfile::TempDir;

#[test]
fn comparing_offsets_is_not_proof_that_acked_events_match_the_original_ones() {
    let temporary = TempDir::new().expect("tempdir");
    let seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());
    let mut inputs = manifest_inputs();
    inputs.audit_coverage = AuditCoverageProof {
        acked_events_checked: 12,
        acked_events_matched: 12,
        offset_comparison_only: true,
    };

    assert!(matches!(
        CheckpointManifest::build(&seal, inputs),
        Err(ManifestError::AuditCoverageNotExact)
    ));
}

#[test]
fn a_partially_matched_acked_event_set_is_refused() {
    let temporary = TempDir::new().expect("tempdir");
    let seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());
    let mut inputs = manifest_inputs();
    inputs.audit_coverage.acked_events_matched = 11;

    assert!(matches!(
        CheckpointManifest::build(&seal, inputs),
        Err(ManifestError::AuditCoverageNotExact)
    ));
}

#[test]
fn missing_package_digests_or_deployment_manifest_is_refused() {
    let temporary = TempDir::new().expect("tempdir");
    let seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());

    let mut without_cli = manifest_inputs();
    without_cli.cli_packages.clear();
    assert!(matches!(
        CheckpointManifest::build(&seal, without_cli),
        Err(ManifestError::PackageDigestsMissing)
    ));

    let mut without_deployment = manifest_inputs();
    without_deployment.deployment.clear();
    assert!(matches!(
        CheckpointManifest::build(&seal, without_deployment),
        Err(ManifestError::DeploymentManifestMissing)
    ));
}

#[test]
fn a_secret_reference_only_names_where_to_fetch_it_never_the_value() {
    let temporary = TempDir::new().expect("tempdir");
    let seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());
    let manifest = CheckpointManifest::build(&seal, manifest_inputs()).expect("manifest");
    let encoded = serde_json::to_string(&manifest).expect("encode");
    assert!(encoded.contains("RESTIC_PASSWORD_FILE"));
    assert!(!encoded.contains("hunter2"));

    for bad in ["restic password = hunter2", "RESTIC PASSWORD", "1BAD"] {
        let mut inputs = manifest_inputs();
        inputs.secret_references = vec![SecretReference {
            logical_name: "restic_repository_password".to_owned(),
            source: SecretSource::EnvironmentVariable {
                variable: bad.to_owned(),
            },
        }];
        assert!(
            matches!(
                CheckpointManifest::build(&seal, inputs),
                Err(ManifestError::SecretReferenceMalformed)
            ),
            "{bad}"
        );
    }
}

#[test]
fn a_persisted_manifest_with_an_unknown_schema_version_fails_closed() {
    let mut manifest = build();
    manifest.schema_version = 99;

    assert!(matches!(
        manifest.validate(),
        Err(ManifestError::UnsupportedSchema { found: 99 })
    ));
}

#[test]
fn the_manifest_round_trips_through_json_without_losing_a_watermark() {
    let manifest = build();
    let encoded = serde_json::to_vec(&manifest).expect("encode");
    let decoded: CheckpointManifest = serde_json::from_slice(&encoded).expect("decode");
    assert_eq!(decoded, manifest);
    assert!(
        encoded
            .windows(DIGEST.len())
            .any(|w| w == DIGEST.as_bytes())
    );
}
