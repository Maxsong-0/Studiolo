//! 实际六 owner 导出接原子封存；所有内容均为专门生成并显式批准 Full 的测试数据。
use super::*;
use crate::checkpoint::CheckpointTimes;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use studiolo_backup::checkpoint::{
    inclusion::{SourceCategory, SourceRecord},
    manifest::{AuditCoverageProof, DeploymentEntry, ManifestInputs, PackageDigest},
    staging::{ObjectRegistration, SealedCheckpoint, SourceCatalog, StagingArea, StagingError},
};
use studiolo_core::{
    BackupScope, Classification, DataLabels, DesktopSync, ModelEgress, Revision, VaultPath,
    VaultUuid, checkpoint::InstantMillis,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_actual_owners_hold_all_gates_through_atomic_readonly_seal() {
    for mode in 0..4 {
        let reject = matches!(mode, 1 | 2);
        let f = Setup::new(true).await;
        let parent = target(&f.fixture, "staging");
        let ephemeral = target(&f.fixture, "ephemeral");
        let sources = [
            "vault", "data", "auth", "control", "agent", "policy", "audit",
        ]
        .map(|name| f.fixture.root.join(name));
        let reader_gid = fs::metadata(&parent).unwrap().gid();
        let area = if mode == 3 {
            fs::set_permissions(&parent, fs::Permissions::from_mode(0o750)).unwrap();
            StagingArea::create_for_reader(
                &parent,
                f.key.checkpoint_id.clone(),
                &sources,
                &ephemeral,
                reader_gid,
                Instant::now() + Duration::from_secs(25),
            )
        } else {
            StagingArea::create(
                &parent,
                f.key.checkpoint_id.clone(),
                &sources,
                &ephemeral,
                Instant::now() + Duration::from_secs(25),
            )
        }
        .unwrap();
        let exports_path = area.export_root().to_owned();
        let frozen = f
            .owners
            .freeze(
                f.request(30_000),
                &f.key,
                BackupTargetKind::LocalTemporaryRepository,
                OwnerExportPaths::under(area.export_root()),
            )
            .unwrap();
        let result = frozen.seal(
            area,
            |root, exported| {
                f.writable(false);
                assert!(
                    f.human
                        .quiesce_operations(Instant::now() + Duration::from_millis(2))
                        .is_err()
                );
                assert!(
                    f.agent
                        .quiesce_operations(Instant::now() + Duration::from_millis(2))
                        .is_err()
                );
                if mode == 1 {
                    return Err(StagingError::Catalog);
                }
                let vault = exported
                    .owners
                    .iter()
                    .find_map(|o| match o.watermark {
                        OwnerWatermark::Broker { vault_uuid, .. } => Some(vault_uuid),
                        _ => None,
                    })
                    .unwrap();
                let mut registrations = Vec::new();
                fixture_records(root, root, vault, &mut registrations);
                if mode == 2 {
                    std::thread::sleep(Duration::from_millis(20));
                }
                SourceCatalog::new(registrations)
            },
            manifest(),
            CheckpointTimes {
                content_cutoff: InstantMillis::from_millis(900),
                now: InstantMillis::from_millis(1000),
                expires_at: InstantMillis::from_millis(if mode == 2 { 1001 } else { 10000 }),
            },
        );
        assert_eq!(result.is_err(), reject);
        if mode == 2 {
            assert!(matches!(result, Err(StagingError::Expired)));
        }
        assert!(!exports_path.exists(), "原始导出必须在成功和失败后都已回收");
        f.writable(false);
        if let Ok(sealed) = result {
            assert_eq!(sealed.manifest().audit_coverage.acked_events_checked, 3);
            let reopened = if mode == 3 {
                SealedCheckpoint::open_shared(
                    &parent,
                    sealed.reference(),
                    reader_gid,
                    InstantMillis::from_millis(1001),
                    Instant::now() + Duration::from_secs(10),
                )
            } else {
                SealedCheckpoint::open(
                    &parent,
                    sealed.reference(),
                    InstantMillis::from_millis(1001),
                    Instant::now() + Duration::from_secs(10),
                )
            };
            assert!(reopened.is_ok());
            let root = &sealed.ticket().transfer_source.root;
            for path in [
                "auth/auth.sqlite3",
                "control/control.sqlite3",
                "agent/agent.sqlite3",
                "policy/policy.sqlite3",
                "audit/audit.sqlite3",
                "agent-audit/audit.sqlite3",
            ] {
                assert!(root.join("data").join(path).is_file(), "{path}");
            }
        }
        drop(frozen);
        f.writable(true);
        assert!(f.human.enter_operation().is_ok());
        assert!(f.agent.enter_operation().is_ok());
        f.workbench.shutdown().await.unwrap();
    }
}
// 这里只是显式批准的虚构样本登记，不能作为生产缺失元数据时的默认值工厂。
fn fixture_records(
    root: &Path,
    directory: &Path,
    vault: VaultUuid,
    records: &mut Vec<ObjectRegistration>,
) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            fixture_records(root, &path, vault, records);
            continue;
        }
        let relative =
            VaultPath::parse(path.strip_prefix(root).unwrap().to_str().unwrap()).unwrap();
        let bytes = fs::read(&path).unwrap();
        records.push(ObjectRegistration {
            export_path: relative.clone(),
            vault_uuid: vault,
            source: SourceRecord {
                path: relative.clone(),
                category: SourceCategory::Canonical,
                labels: Some(
                    DataLabels::new(
                        Classification::Restricted,
                        BackupScope::Full,
                        DesktopSync::Deny,
                        ModelEgress::Deny,
                    )
                    .unwrap(),
                ),
                expires_at: None,
                crypto_erased: false,
                declared_revision: Some(Revision::compute(vault, &relative, &bytes)),
                observed_revision: None,
            },
            redacted: None,
        });
    }
}
fn manifest() -> ManifestInputs {
    let package = |name: &str| PackageDigest {
        name: name.into(),
        version: "fixture-1".into(),
        digest: "ab".repeat(32),
    };
    ManifestInputs {
        application_packages: vec![package("studiolo")],
        cli_packages: vec![package("restic")],
        deployment: vec![DeploymentEntry {
            unit: "fixture".into(),
            revision: "fixture-1".into(),
        }],
        secret_references: Vec::new(),
        // 实际总窗口必须用原始核验次数覆盖该占位，不能把此值作为凭证。
        audit_coverage: AuditCoverageProof {
            acked_events_checked: 0,
            acked_events_matched: 0,
            offset_comparison_only: true,
        },
    }
}
