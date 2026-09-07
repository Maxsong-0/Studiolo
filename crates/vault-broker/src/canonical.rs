//! Existing canonical identity and bounded content access without write authority.

use std::path::{Path, PathBuf};
use studiolo_core::{Revision, VaultPath, VaultUuid};

use crate::{BrokerError, ReadOnlyVault, VaultIdentity};

/// One stable canonical read; no index generation is used as an editing revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalRead {
    /// Persistent identity read from the existing canonical marker.
    pub vault_uuid: VaultUuid,
    /// Logical path beneath the approved canonical root.
    pub path: VaultPath,
    /// Exact bytes; text encoding and rendering are the caller's responsibility.
    pub content: Vec<u8>,
    /// Existing Broker revision algorithm, bound to Vault, path and exact bytes.
    pub revision: Revision,
}

/// Narrow read handle for one approved, existing canonical root.
///
/// This handle creates no marker, data directory, history, snapshot or cache and
/// has no mutation capability. It rechecks the selected root and its bounded
/// identity record before and after each read. These checks detect observed
/// replacement; they do not lock out unrelated filesystem writers.
pub struct CanonicalReadVault {
    reader: ReadOnlyVault,
    selected_root: PathBuf,
    vault_uuid: VaultUuid,
}

impl CanonicalReadVault {
    /// Opens an existing canonical root approved by the trusted composition root.
    ///
    /// Requires an absolute path and an existing format-v1 `.workbench/vault.json`
    /// of at most 4096 bytes. Missing or invalid state is never initialized or
    /// repaired. The caller must not obtain this path from an HTTP request.
    pub fn open_existing(root: &Path) -> Result<Self, BrokerError> {
        if !root.is_absolute() {
            return Err(BrokerError::io(
                "open canonical root",
                std::io::Error::other("an approved absolute root is required"),
            ));
        }
        let reader = ReadOnlyVault::open(root)?;
        let vault_uuid = identity(&reader)?;
        let handle = Self {
            reader,
            selected_root: root.to_owned(),
            vault_uuid,
        };
        handle.check_binding()?;
        Ok(handle)
    }

    /// Returns the pinned persistent Vault identity, not a freshness assertion.
    #[must_use]
    pub fn vault_uuid(&self) -> VaultUuid {
        self.vault_uuid
    }

    /// Returns the still-bound directory identity for trusted startup composition.
    /// This value is not intended for HTTP responses or caller authentication.
    #[cfg(unix)]
    pub fn root_identity(&self) -> Result<(u64, u64), BrokerError> {
        self.check_binding()?;
        self.reader.root_identity()
    }

    /// Reads a visible regular file under a caller-supplied byte ceiling.
    ///
    /// The caller must impose its endpoint's maximum; zero allows only empty
    /// files. Growth beyond the ceiling is rejected, retaining at most ceiling
    /// plus one content bytes. Hidden paths, links and special files are refused.
    /// Stable single-file content does not assert that a derived index is fresh.
    pub fn read_file(
        &self,
        path: &VaultPath,
        max_bytes: u64,
    ) -> Result<CanonicalRead, BrokerError> {
        self.check_binding()?;
        let note = self
            .reader
            .read_in_namespace(path, max_bytes, self.vault_uuid)?;
        self.check_binding()?;
        Ok(CanonicalRead {
            vault_uuid: self.vault_uuid,
            path: note.path,
            content: note.content,
            revision: note.revision,
        })
    }

    fn check_binding(&self) -> Result<(), BrokerError> {
        self.check_root()?;
        if identity(&self.reader)? != self.vault_uuid {
            return Err(BrokerError::CorruptIdentity);
        }
        self.check_root()
    }

    #[cfg(unix)]
    fn check_root(&self) -> Result<(), BrokerError> {
        let current =
            ReadOnlyVault::open(&self.selected_root).map_err(|_| BrokerError::RootChanged)?;
        if current
            .root_identity()
            .map_err(|_| BrokerError::RootChanged)?
            != self
                .reader
                .root_identity()
                .map_err(|_| BrokerError::RootChanged)?
        {
            return Err(BrokerError::RootChanged);
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn check_root(&self) -> Result<(), BrokerError> {
        Err(BrokerError::RootChanged)
    }
}

fn identity(reader: &ReadOnlyVault) -> Result<VaultUuid, BrokerError> {
    let bytes = reader.read_identity().map_err(|error| match error {
        BrokerError::NotFound(_)
        | BrokerError::ReadLimitExceeded { .. }
        | BrokerError::UnsafeNode { .. } => BrokerError::CorruptIdentity,
        other => other,
    })?;
    VaultIdentity::decode(&bytes).map_err(|_| BrokerError::CorruptIdentity)
}
