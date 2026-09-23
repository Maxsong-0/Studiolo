//! 清单契约：各 owner 水位各有含义，缺一项、被别的计数顶替或秘密泄漏都要拒绝。
//!
//! 本文件管各 owner 水位（含 M4 策略库）；固定事实与持久清单的 schema／往返在 `facts`。

mod facts;

use super::support::{DIGEST, manifest_inputs, ticket};
use studiolo_backup::checkpoint::manifest::{CheckpointManifest, ManifestError};
use studiolo_backup::checkpoint::types::OwnerRoster;
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

    assert_eq!(manifest.watermarks.len(), 7);
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
fn a_policy_effective_hash_must_be_a_real_digest_and_a_bare_version_number_is_a_substitution() {
    let temporary = TempDir::new().expect("tempdir");
    let uppercase = DIGEST.to_ascii_uppercase();
    // effective 版本号就是「一个数」；空串、截断、大写十六进制只是格式不合法。
    for (broken, substituted) in [
        ("5", true),
        ("", false),
        (&DIGEST[..63], false),
        (uppercase.as_str(), false),
    ] {
        let mut seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());
        for sealed in &mut seal.owners {
            if let OwnerWatermark::Policy { effective_hash, .. } = &mut sealed.watermark {
                *effective_hash = broken.to_owned();
            }
        }
        let expected = if substituted {
            ManifestError::WatermarkSubstituted {
                owner: CheckpointOwner::Policy,
                field: "effective_hash",
            }
        } else {
            ManifestError::WatermarkMalformed {
                owner: CheckpointOwner::Policy,
                field: "effective_hash",
            }
        };
        assert_eq!(
            CheckpointManifest::build(&seal, manifest_inputs()).err(),
            Some(expected),
            "{broken:?}"
        );
    }
}

#[test]
fn a_persisted_manifest_rechecks_the_policy_watermark_on_validation() {
    let mut manifest = build();
    assert!(
        manifest
            .watermarks
            .iter()
            .any(|watermark| watermark.owner() == CheckpointOwner::Policy),
        "策略库水位必须进封存清单"
    );
    for watermark in &mut manifest.watermarks {
        if let OwnerWatermark::Policy {
            effective_version,
            effective_hash,
            ..
        } = watermark
        {
            *effective_hash = effective_version.to_string();
        }
    }

    assert_eq!(
        manifest.validate(),
        Err(ManifestError::WatermarkSubstituted {
            owner: CheckpointOwner::Policy,
            field: "effective_hash"
        })
    );
}

#[test]
fn a_present_policy_store_needs_its_own_watermark_and_an_absent_one_needs_none() {
    let temporary = TempDir::new().expect("tempdir");
    let mut seal = ticket(VaultUuid::new(), VaultUuid::new(), temporary.path());
    seal.owners
        .retain(|sealed| sealed.owner != CheckpointOwner::Policy);
    assert_eq!(
        CheckpointManifest::build(&seal, manifest_inputs()).err(),
        Some(ManifestError::WatermarkMissing {
            owner: CheckpointOwner::Policy
        })
    );

    seal.roster = OwnerRoster::new(seal.roster.entries().map(|(owner, assembly)| {
        if owner == CheckpointOwner::Policy {
            (owner, OwnerAssembly::Absent)
        } else {
            (owner, assembly)
        }
    }))
    .expect("roster");
    let manifest =
        CheckpointManifest::build(&seal, manifest_inputs()).expect("未装配的策略库不需要水位");
    assert_eq!(
        manifest.roster.assembly(CheckpointOwner::Policy),
        OwnerAssembly::Absent
    );
    manifest.validate().expect("显式 absent 的清单可再次校验");
}

#[test]
fn a_persisted_roster_that_predates_the_policy_store_fails_closed_instead_of_reading_as_absent() {
    let mut encoded = serde_json::to_value(build()).expect("encode");
    // 模拟策略库纳入之前写下的清单：roster 少一项，也没有策略库水位。
    encoded["roster"]
        .as_array_mut()
        .expect("roster")
        .retain(|entry| entry[0] != "policy");
    encoded["watermarks"]
        .as_array_mut()
        .expect("watermarks")
        .retain(|watermark| watermark["owner"] != "policy");

    assert!(
        serde_json::from_value::<CheckpointManifest>(encoded).is_err(),
        "缺项的 owner 清单不能被静默读成 absent"
    );
}
