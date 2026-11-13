//! 主账本与接入方账本独立，构造和恢复校验不能走两套门槛。
use super::build;
use studiolo_backup::checkpoint::{
    manifest::{CheckpointManifest, ManifestError},
    types::OwnerRoster,
};
use studiolo_core::{
    VaultUuid,
    checkpoint::{CheckpointOwner, OwnerAssembly, OwnerWatermark},
};

#[test]
fn agent_store_requires_its_own_audit_cutoff_even_when_primary_offset_is_larger() {
    let mut manifest = build();
    for watermark in &mut manifest.watermarks {
        if let OwnerWatermark::Audit {
            last_offset,
            agent_ledger,
            ..
        } = watermark
        {
            *last_offset = 10000;
            *agent_ledger = None;
        }
    }
    assert_eq!(manifest.validate(), Err(ManifestError::AuditLayoutMismatch));
    let encoded = serde_json::to_vec(&manifest).unwrap();
    let reopened: CheckpointManifest = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(reopened.validate(), Err(ManifestError::AuditLayoutMismatch));
}
#[test]
fn absent_agent_store_must_not_claim_an_agent_ledger() {
    let mut manifest = build();
    manifest.roster = OwnerRoster::new(manifest.roster.entries().map(|(owner, assembly)| {
        (
            owner,
            if owner == CheckpointOwner::AgentStore {
                OwnerAssembly::Absent
            } else {
                assembly
            },
        )
    }))
    .unwrap();
    manifest
        .watermarks
        .retain(|w| w.owner() != CheckpointOwner::AgentStore);
    assert_eq!(manifest.validate(), Err(ManifestError::AuditLayoutMismatch));
    for watermark in &mut manifest.watermarks {
        if let OwnerWatermark::Audit { agent_ledger, .. } = watermark {
            *agent_ledger = None;
        }
    }
    manifest.validate().unwrap();
}
#[test]
fn auth_anchor_cannot_point_past_primary_cutoff_or_into_the_agent_ledger() {
    let mut manifest = build();
    for watermark in &mut manifest.watermarks {
        if let OwnerWatermark::Audit {
            last_offset,
            agent_ledger,
            ..
        } = watermark
        {
            *last_offset = 90;
            agent_ledger.as_mut().unwrap().last_offset = 10000;
        }
    }
    assert_eq!(manifest.validate(), Err(ManifestError::AuditLayoutMismatch));
}
#[test]
fn duplicate_absent_and_cross_vault_watermarks_are_rejected() {
    let mut manifest = build();
    manifest.watermarks.push(manifest.watermarks[0].clone());
    assert!(matches!(
        manifest.validate(),
        Err(ManifestError::UnexpectedWatermark { .. })
    ));
    let mut manifest = build();
    manifest.roster = OwnerRoster::new(manifest.roster.entries().map(|(owner, assembly)| {
        (
            owner,
            if owner == CheckpointOwner::Policy {
                OwnerAssembly::Absent
            } else {
                assembly
            },
        )
    }))
    .unwrap();
    assert_eq!(
        manifest.validate(),
        Err(ManifestError::UnexpectedWatermark {
            owner: CheckpointOwner::Policy
        })
    );
    let mut manifest = build();
    for watermark in &mut manifest.watermarks {
        if let OwnerWatermark::Control { vault_uuid, .. } = watermark {
            *vault_uuid = VaultUuid::new();
        }
    }
    assert_eq!(
        manifest.validate(),
        Err(ManifestError::VaultBindingMismatch)
    );
}
#[test]
fn a_previous_delivered_sequence_does_not_excuse_any_pending_control_outbox() {
    let mut manifest = build();
    for watermark in &mut manifest.watermarks {
        if let OwnerWatermark::Control {
            outbox_pending,
            outbox_delivered_through,
            ..
        } = watermark
        {
            *outbox_pending = 1;
            *outbox_delivered_through = 100;
        }
    }
    assert_eq!(
        manifest.validate(),
        Err(ManifestError::ControlOutboxUnsettled)
    );
}
#[test]
fn restored_manifest_rechecks_packages_deployment_and_secret_references() {
    for change in 0..4 {
        let mut manifest = build();
        match change {
            0 => manifest.application_packages.clear(),
            1 => manifest.cli_packages[0].digest = "wrong".into(),
            2 => manifest.deployment[0].revision.clear(),
            _ => manifest.secret_references[0].logical_name.clear(),
        }
        let reopened: CheckpointManifest =
            serde_json::from_slice(&serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(reopened.validate().is_err());
    }
}
#[test]
fn unknown_watermark_fields_and_malformed_secondary_hashes_fail_closed() {
    let mut encoded = serde_json::to_value(build()).unwrap();
    encoded["watermarks"][0]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<CheckpointManifest>(encoded).is_err());
    let mut manifest = build();
    for watermark in &mut manifest.watermarks {
        if let OwnerWatermark::Audit { agent_ledger, .. } = watermark {
            agent_ledger.as_mut().unwrap().last_hash = "wrong".into();
        }
    }
    assert_eq!(
        manifest.validate(),
        Err(ManifestError::WatermarkMalformed {
            owner: CheckpointOwner::Audit,
            field: "agent_last_hash"
        })
    );
}
