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
async fn checkpoint_actual_owners_release_every_gate_before_the_atomic_readonly_seal() {
    // 第 4 种不用夹具登记，走生产用的来源目录：Broker 扫描这份真实导出，备份层组装。
    for mode in 0..5 {
        let reject = matches!(mode, 1 | 2);
        let f = Setup::new(true).await;
        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
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
        f.writable(false);
        // 封存（导出、登记、过滤视图、发布）在放开全部屏障之后做（BUG-045）：这时各库照常可写，
        // 两边的完整操作都排得空。
        let mut released = frozen
            .release(Instant::now() + Duration::from_secs(30))
            .unwrap();
        let result = released.seal(
            area,
            |root, exported| {
                f.writable(true);
                assert!(
                    f.human
                        .quiesce_operations(Instant::now() + Duration::from_millis(200))
                        .is_ok()
                );
                assert!(
                    f.agent
                        .quiesce_operations(Instant::now() + Duration::from_millis(200))
                        .is_ok()
                );
                if mode == 1 {
                    return Err(StagingError::Catalog);
                }
                if mode == 4 {
                    let catalog = crate::checkpoint::trusted_catalog(root, exported);
                    if catalog.is_err() {
                        // 扫描器和真实导出布局对不上时，把布局打出来，不用再猜是哪个文件。
                        let mut layout = Vec::new();
                        listing(root, root, &mut layout);
                        layout.sort();
                        eprintln!("真实导出被来源扫描拒绝，文件清单：{layout:#?}");
                    }
                    return catalog;
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
                content_cutoff: InstantMillis::from_millis(now - 100),
                now: InstantMillis::from_millis(now),
                expires_at: InstantMillis::from_millis(now + if mode == 2 { 1 } else { 30_000 }),
            },
        );
        assert_eq!(result.is_err(), reject);
        if mode == 2 {
            assert!(matches!(result, Err(StagingError::Expired)));
        }
        assert!(!exports_path.exists(), "原始导出必须在成功和失败后都已回收");
        f.writable(true);
        if let Ok(sealed) = &result {
            assert_eq!(sealed.manifest().audit_coverage.acked_events_checked, 3);
            let reopened = if mode == 3 {
                SealedCheckpoint::open_shared(
                    &parent,
                    sealed.reference(),
                    reader_gid,
                    InstantMillis::from_millis(now + 1),
                    Instant::now() + Duration::from_secs(10),
                )
            } else {
                SealedCheckpoint::open(
                    &parent,
                    sealed.reference(),
                    InstantMillis::from_millis(now + 1),
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
        drop(released);
        f.writable(true);
        assert!(f.human.enter_operation().is_ok());
        assert!(f.agent.enter_operation().is_ok());
        if let Ok(sealed) = result {
            use crate::backup::{admission::BackupAdmission, audit::BackupAuditPort};
            let approved = BackupAdmission::approve(
                f.human.clone(),
                f.key.clone(),
                BackupTargetKind::LocalTemporaryRepository,
                &sealed,
            )
            .unwrap();
            assert_eq!(approved.source(), sealed.reference());
            assert!(approved.check_live().is_ok());
            // 原封存同键重放不再追加，也不能拿另一条审计链来认领封存。
            let before = f
                .human
                .query(&studiolo_audit_ledger::AuditFilter::default())
                .unwrap();
            assert!(
                BackupAdmission::approve(
                    f.human.clone(),
                    f.key.clone(),
                    BackupTargetKind::LocalTemporaryRepository,
                    &sealed
                )
                .is_ok()
            );
            assert_eq!(
                before,
                f.human
                    .query(&studiolo_audit_ledger::AuditFilter::default())
                    .unwrap()
            );
            let wrong =
                Arc::new(SqliteAuditLedger::open(&target(&f.fixture, "wrong-audit")).unwrap());
            wrong
                .append(backup_intent(&f.key, BackupTargetKind::LocalTemporaryRepository).unwrap())
                .unwrap();
            assert!(
                BackupAdmission::approve(
                    wrong,
                    f.key.clone(),
                    BackupTargetKind::LocalTemporaryRepository,
                    &sealed
                )
                .is_err()
            );
            let resumed = BackupAdmission::resume(
                f.human.clone(),
                &f.key,
                BackupTargetKind::LocalTemporaryRepository,
            )
            .unwrap();
            let port = BackupAuditPort::new(resumed);
            let request = studiolo_backup::worker::audit::BackupAuditRequest::read(
                f.key.clone(),
                BackupTargetKind::LocalTemporaryRepository,
                studiolo_audit_ledger::AuditPhase::Intent,
            );
            assert!(
                request
                    .decode_reply(&port.dispatch(&request.encode().unwrap()).unwrap())
                    .unwrap()
                    .is_some()
            );
        }
        f.workbench.shutdown().await.unwrap();
    }
}
fn listing(root: &Path, directory: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            listing(root, &path, files);
        } else {
            files.push(path.strip_prefix(root).unwrap().display().to_string());
        }
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
                source_policy: None,
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
pub(super) fn manifest() -> ManifestInputs {
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
