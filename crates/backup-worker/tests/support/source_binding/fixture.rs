use std::{
    fs,
    num::NonZeroU64,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use studiolo_backup::checkpoint::{
    inclusion::{SourceCategory, SourceRecord},
    manifest::{AuditCoverageProof, DeploymentEntry, ManifestInputs, PackageDigest},
    staging::{ObjectRegistration, SealInputs, SourceCatalog, StagingArea, StagingError},
    types::OwnerRoster,
};
use studiolo_backup::{
    checkpoint::staging::{SealedCheckpoint, SealedReference},
    worker::{
        restic::{
            CredentialHolder, JobTag, RepositoryRef, ResticCommand, ResticIdentity,
            ResticInvocation,
        },
        state::JobKey,
    },
};
use studiolo_core::{
    BackupScope, Classification, DataLabels, DesktopSync, ModelEgress, Revision, VaultPath,
    VaultUuid,
    checkpoint::{
        CheckpointId, CheckpointOwner, InstantMillis, OwnerAssembly, OwnerWatermark,
        SealedOwnerCheckpoint, WalCheckpointProof,
    },
};

pub(crate) struct Fixture {
    _temp: tempfile::TempDir,
    pub parent: PathBuf,
    source: PathBuf,
    ephemeral: PathBuf,
    group: u32,
}
impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("sealed");
        let source = temp.path().join("source");
        let ephemeral = temp.path().join("ephemeral");
        for (path, mode) in [(&parent, 0o750), (&source, 0o700), (&ephemeral, 0o700)] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        }
        let group = fs::metadata(&parent).unwrap().gid();
        Self {
            _temp: temp,
            parent,
            source,
            ephemeral,
            group,
        }
    }
    pub fn seal(&self, id: &str) -> SealedCheckpoint {
        let area = StagingArea::create_for_reader(
            &self.parent,
            CheckpointId::parse(id).unwrap(),
            std::slice::from_ref(&self.source),
            &self.ephemeral,
            self.group,
            deadline(),
        )
        .unwrap();
        let bytes = b"fixture content";
        let path = VaultPath::parse("canonical/note").unwrap();
        let vault = VaultUuid::new();
        let file = area.export_root().join(path.as_str());
        fs::write(&file, bytes).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let source = SourceRecord {
            path: path.clone(),
            category: SourceCategory::SourceMaterial,
            source_policy: Some(
                studiolo_core::SourcePolicyRef::new("fixture-source", "ab".repeat(32)).unwrap(),
            ),
            labels: Some(
                DataLabels::new(
                    Classification::Restricted,
                    BackupScope::Full,
                    DesktopSync::Deny,
                    ModelEgress::Deny,
                )
                .unwrap(),
            ),
            expires_at: Some(ms(10000)),
            crypto_erased: false,
            declared_revision: Some(Revision::compute(vault, &path, bytes)),
            observed_revision: None,
        };
        let catalog = SourceCatalog::new([ObjectRegistration {
            export_path: path,
            vault_uuid: vault,
            source,
            redacted: None,
        }])
        .unwrap();
        let digest = "ab".repeat(32);
        let owner = SealedOwnerCheckpoint {
            owner: CheckpointOwner::Audit,
            schema_version: 1,
            wal: WalCheckpointProof {
                busy: 0,
                log_frames: 0,
                checkpointed_frames: 0,
            },
            target_synced: true,
            parent_directory_synced: true,
            reopen_verified: true,
            source_binding_stable: true,
            watermark: OwnerWatermark::Audit {
                last_offset: 1,
                last_hash: digest.clone(),
                agent_ledger: None,
            },
        };
        let roster = OwnerRoster::new(CheckpointOwner::FREEZE_ORDER.iter().map(|owner| {
            (
                *owner,
                if *owner == CheckpointOwner::Audit {
                    OwnerAssembly::Present
                } else {
                    OwnerAssembly::Absent
                },
            )
        }))
        .unwrap();
        let package = PackageDigest {
            name: "fixture".into(),
            version: "1".into(),
            digest,
        };
        let manifest = ManifestInputs {
            application_packages: vec![package.clone()],
            cli_packages: vec![package],
            deployment: vec![DeploymentEntry {
                unit: "fixture.service".into(),
                revision: "r1".into(),
            }],
            secret_references: vec![],
            audit_coverage: AuditCoverageProof {
                acked_events_checked: 1,
                acked_events_matched: 1,
                offset_comparison_only: false,
            },
        };
        area.seal(
            &catalog,
            SealInputs {
                roster,
                owners: vec![owner],
                manifest,
                content_cutoff: ms(900),
                now: ms(1000),
                expires_at: ms(9000),
            },
        )
        .unwrap()
    }
    pub fn reopen(
        &self,
        reference: &SealedReference,
    ) -> std::result::Result<SealedCheckpoint, StagingError> {
        SealedCheckpoint::open_shared(&self.parent, reference, self.group, ms(1100), deadline())
    }
}
pub(crate) fn ms(value: i64) -> InstantMillis {
    InstantMillis::from_millis(value)
}
pub(crate) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(20)
}
pub(crate) fn repo() -> RepositoryRef {
    RepositoryRef::parse("rest:https://backup.example.test/repository/").unwrap()
}
pub(crate) fn job(id: &str, sequence: u64) -> JobKey {
    JobKey {
        sequence: NonZeroU64::new(sequence).unwrap(),
        checkpoint_id: CheckpointId::parse(id).unwrap(),
    }
}
pub(crate) fn plan(repository: RepositoryRef, key: &JobKey, root: &Path) -> ResticInvocation {
    ResticInvocation::plan(
        repository,
        &ResticCommand::Backup {
            tag: JobTag::for_job(key).unwrap(),
            source_root: root.to_owned(),
        },
        CredentialHolder::BackupWorker,
        ResticIdentity::Append,
    )
    .unwrap()
}
