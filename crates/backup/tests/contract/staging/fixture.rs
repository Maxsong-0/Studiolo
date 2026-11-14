use super::super::support;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use studiolo_backup::checkpoint::{
    inclusion::{SourceCategory, SourceRecord},
    staging::{ObjectRegistration, RedactedObject, SealInputs, StagingArea, StagingError},
};
use studiolo_core::{
    BackupScope, Classification, DataLabels, DesktopSync, ModelEgress, Revision, VaultPath,
    VaultUuid,
    checkpoint::{CheckpointId, InstantMillis},
};
pub struct Fixture {
    pub temp: tempfile::TempDir,
    pub parent: PathBuf,
    pub source: PathBuf,
    pub ephemeral: PathBuf,
    pub vault: VaultUuid,
}
impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("staging");
        let source = temp.path().join("source");
        let ephemeral = temp.path().join("ephemeral");
        for path in [&parent, &source, &ephemeral] {
            private_dir(path);
        }
        Self {
            temp,
            parent,
            source,
            ephemeral,
            vault: VaultUuid::new(),
        }
    }
    pub fn try_area(&self, id: &str) -> Result<StagingArea, StagingError> {
        StagingArea::create(
            &self.parent,
            checkpoint(id),
            std::slice::from_ref(&self.source),
            &self.ephemeral,
            deadline(),
        )
    }
    pub fn area(&self, id: &str) -> StagingArea {
        self.try_area(id).unwrap()
    }
    pub fn object(
        &self,
        area: &StagingArea,
        path: &str,
        bytes: &[u8],
        scope: BackupScope,
    ) -> ObjectRegistration {
        private_file(&area.export_root().join(path), bytes);
        let path = VaultPath::parse(path).unwrap();
        ObjectRegistration {
            export_path: path.clone(),
            vault_uuid: self.vault,
            source: SourceRecord {
                path: path.clone(),
                category: SourceCategory::SourceMaterial,
                source_policy: Some(
                    studiolo_core::SourcePolicyRef::new("fixture-source", "ab".repeat(32)).unwrap(),
                ),
                labels: Some(
                    DataLabels::new(
                        Classification::Restricted,
                        scope,
                        DesktopSync::Deny,
                        ModelEgress::Deny,
                    )
                    .unwrap(),
                ),
                expires_at: Some(InstantMillis::from_millis(10000)),
                crypto_erased: false,
                declared_revision: Some(Revision::compute(self.vault, &path, bytes)),
                observed_revision: None,
            },
            redacted: None,
        }
    }
    pub fn inputs(&self) -> SealInputs {
        let ticket = support::ticket(self.vault, VaultUuid::new(), &self.parent);
        SealInputs {
            roster: ticket.roster,
            owners: ticket.owners,
            manifest: support::manifest_inputs(),
            content_cutoff: InstantMillis::from_millis(900),
            now: InstantMillis::from_millis(1000),
            expires_at: InstantMillis::from_millis(9000),
        }
    }
}
pub fn checkpoint(id: &str) -> CheckpointId {
    CheckpointId::parse(id).unwrap()
}
pub fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
pub fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
pub fn private_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
pub fn redacted(bytes: &[u8]) -> RedactedObject {
    RedactedObject {
        bytes: bytes.into(),
        sha256: Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    }
}
