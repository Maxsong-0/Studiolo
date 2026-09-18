//! 清单契约：各 owner 水位各有含义，缺一项、被别的计数顶替或秘密泄漏都要拒绝。

use super::support::{DIGEST, manifest_inputs, ticket};
use studiolo_backup::checkpoint::manifest::{
    AuditCoverageProof, CheckpointManifest, ManifestError, SecretReference, SecretSource,
};
use studiolo_core::VaultUuid;
use studiolo_core::checkpoint::{CheckpointOwner, OwnerAssembly, OwnerWatermark};
use tempfile::TempDir;

fn build() -> CheckpointManifest {
    let temporary = TempDir::new().expect("tempdir");
    CheckpointManifest::build(
        &ticket(VaultUuid::new(), VaultUuid::new(), temporary.path()),
        manifest_inputs(),
    )
    .expect("manifest")
}

#[test]
fn the_manifest_records_every_required_watermark_for_one_checkpoint_id() {
    let manifest = build();

    assert_eq!(manifest.watermarks.len(), 6);
    for owner in CheckpointOwner::FREEZE_ORDER {
        assert!(
            manifest
                .watermarks
                .iter()
                .any(|watermark| watermark.owner() == owner),
            "{owner:?} 的水位必须在清单里"
        );
        assert_eq!(manifest.roster.assembly(owner), OwnerAssembly::Present);
    }
    assert!(!manifest.application_packages.is_empty());
    assert!(!manifest.cli_packages.is_empty());
    assert!(!manifest.deployment.is_empty());
    manifest.validate().expect("已持久清单可再次校验");
}

#[test]
fn a_present_owner_without_its_own_watermark_is_refused() {
    let temporary = TempDir::new().expect("tempdir");
    let mut seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());
    seal.owners
        .retain(|owner| owner.owner != CheckpointOwner::Bridge);

    assert!(matches!(
        CheckpointManifest::build(&seal, manifest_inputs()),
        Err(ManifestError::WatermarkMissing {
            owner: CheckpointOwner::Bridge
        })
    ));
}

#[test]
fn an_index_generation_number_cannot_stand_in_for_the_bridge_common_baseline() {
    let temporary = TempDir::new().expect("tempdir");
    let mut seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());
    for sealed in &mut seal.owners {
        if let OwnerWatermark::Bridge {
            common_base_digest, ..
        } = &mut sealed.watermark
        {
            // 索引 generation 就是这种「一个数」。
            *common_base_digest = "17".to_owned();
        }
    }

    assert!(matches!(
        CheckpointManifest::build(&seal, manifest_inputs()),
        Err(ManifestError::WatermarkSubstituted {
            owner: CheckpointOwner::Bridge,
            field: "common_base_digest"
        })
    ));
}

#[test]
fn the_last_sync_exit_timestamp_cannot_stand_in_for_the_observation_sequence() {
    let temporary = TempDir::new().expect("tempdir");
    let mut seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());
    for sealed in &mut seal.owners {
        if let OwnerWatermark::Bridge {
            observation_sequence,
            durable_gate_closed,
            ..
        } = &mut sealed.watermark
        {
            *observation_sequence = 0;
            *durable_gate_closed = false;
        }
    }

    assert!(matches!(
        CheckpointManifest::build(&seal, manifest_inputs()),
        Err(ManifestError::BridgeGateOpen)
    ));
}

#[test]
fn the_audit_chain_hash_must_be_a_real_digest_and_a_bare_number_is_named_a_substitution() {
    let temporary = TempDir::new().expect("tempdir");
    // 空串与随手写的字符串是「格式不合法」；纯十进制数字是「被别的计数顶替」。
    for (broken, substituted) in [("", false), ("not-a-digest", false), ("1234", true)] {
        let mut seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());
        for sealed in &mut seal.owners {
            if let OwnerWatermark::Audit { last_hash, .. } = &mut sealed.watermark {
                *last_hash = broken.to_owned();
            }
        }
        let error = CheckpointManifest::build(&seal, manifest_inputs()).err();
        if substituted {
            assert!(
                matches!(
                    error,
                    Some(ManifestError::WatermarkSubstituted {
                        owner: CheckpointOwner::Audit,
                        field: "last_hash"
                    })
                ),
                "{broken}"
            );
        } else {
            assert!(
                matches!(
                    error,
                    Some(ManifestError::WatermarkMalformed {
                        owner: CheckpointOwner::Audit,
                        ..
                    })
                ),
                "{broken}"
            );
        }
    }
}

#[test]
fn an_auth_owner_binding_digest_that_is_just_a_counter_is_refused_the_same_way() {
    let temporary = TempDir::new().expect("tempdir");
    let mut seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());
    for sealed in &mut seal.owners {
        if let OwnerWatermark::Auth {
            owner_binding_digest,
            ..
        } = &mut sealed.watermark
        {
            *owner_binding_digest = "1758067200000".to_owned();
        }
    }

    assert!(matches!(
        CheckpointManifest::build(&seal, manifest_inputs()),
        Err(ManifestError::WatermarkSubstituted {
            owner: CheckpointOwner::Auth,
            field: "owner_binding_digest"
        })
    ));
}

#[test]
fn a_broker_watermark_with_unresolved_work_cannot_enter_the_manifest() {
    let temporary = TempDir::new().expect("tempdir");
    let mut seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());
    for sealed in &mut seal.owners {
        if let OwnerWatermark::Broker { unresolved, .. } = &mut sealed.watermark {
            *unresolved = true;
        }
    }

    assert!(matches!(
        CheckpointManifest::build(&seal, manifest_inputs()),
        Err(ManifestError::BrokerUnresolved)
    ));
}

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
