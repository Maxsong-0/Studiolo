//! 把私有快照读到内存并绑定批准对象；不提供任意路径递归拷贝。

use crate::{Error, Options, PlanSummary};
use rustix::fs::{Mode, OFlags, open};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use studiolo_core::VaultPath;
use studiolo_host_config::{CanonicalServingRoots, Config};
use studiolo_vault_broker::{ReadEntryKind, ReadNote, ReadOnlyVault, ScanLimits};

const LIMITS: ScanLimits = ScanLimits {
    max_file_bytes: 4 * 1024 * 1024,
    max_total_bytes: 64 * 1024 * 1024,
    max_entries: 20_000,
    max_depth: 32,
};

#[derive(Clone, Serialize)]
struct RootIdentity {
    name: &'static str,
    path: PathBuf,
    device: u64,
    inode: u64,
}

struct Pin {
    record: RootIdentity,
    held: ReadOnlyVault,
    private: bool,
}

impl Pin {
    fn new(name: &'static str, path: &Path, private: bool) -> Result<Self, Error> {
        clean_absolute(path)?;
        let held = Self::open(path, private)?;
        let (device, inode) = held.root_identity().map_err(|_| Error::Paths)?;
        Ok(Self {
            record: RootIdentity {
                name,
                path: path.into(),
                device,
                inode,
            },
            held,
            private,
        })
    }

    fn open(path: &Path, private: bool) -> Result<ReadOnlyVault, Error> {
        if private {
            ReadOnlyVault::open_private(path)
        } else {
            ReadOnlyVault::open(path)
        }
        .map_err(|_| Error::Paths)
    }

    fn check(&self) -> Result<(), Error> {
        let actual = Self::open(&self.record.path, self.private)?;
        let identity = (self.record.device, self.record.inode);
        if actual.root_identity().map_err(|_| Error::Changed)? != identity
            || self.held.root_identity().map_err(|_| Error::Changed)? != identity
        {
            return Err(Error::Changed);
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
struct ConfigRevision {
    path: PathBuf,
    stamp: (u64, u64, u64, i64, i64, i64, i64),
    sha256: String,
}

impl ConfigRevision {
    fn read(path: &Path) -> Result<Self, Error> {
        let fd = open(
            path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Error::Paths)?;
        let file = File::from(fd);
        let before = file.metadata().map_err(|_| Error::Paths)?;
        if !before.is_file()
            || before.nlink() != 1
            || before.len() > 16_384
            || before.mode() & 0o7777 != 0o600
            || before.uid() != rustix::process::getuid().as_raw()
        {
            return Err(Error::Paths);
        }
        let mut bytes = Vec::new();
        (&file)
            .take(16_385)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Paths)?;
        let after = file.metadata().map_err(|_| Error::Paths)?;
        let path_after = fs::symlink_metadata(path).map_err(|_| Error::Paths)?;
        let stamp = |m: &fs::Metadata| {
            (
                m.dev(),
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        };
        if bytes.len() as u64 != before.len()
            || stamp(&before) != stamp(&after)
            || stamp(&before) != stamp(&path_after)
            || !path_after.is_file()
        {
            return Err(Error::Changed);
        }
        Ok(Self {
            path: path.into(),
            stamp: stamp(&before),
            sha256: digest(&bytes),
        })
    }
}

#[derive(Serialize)]
struct FileDigest<'a> {
    path: &'a VaultPath,
    bytes: usize,
    sha256: String,
}

#[derive(Serialize)]
struct DigestInput<'a> {
    domain: &'static str,
    roots: Vec<&'a RootIdentity>,
    config_revision: &'a ConfigRevision,
    allow: &'a [VaultPath],
    files: Vec<FileDigest<'a>>,
}

pub(crate) struct Prepared {
    pub(crate) options: Options,
    pub(crate) config: Config,
    pub(crate) summary: PlanSummary,
    pub(crate) notes: Vec<ReadNote>,
    approved: CanonicalServingRoots,
    pins: Vec<Pin>,
    revision: ConfigRevision,
}

impl Prepared {
    pub(crate) fn capture(options: &Options) -> Result<Self, Error> {
        let mut options = options.clone();
        if options.allow.is_empty() || options.allow.len() > 64 {
            return Err(Error::Arguments);
        }
        options.allow.sort();
        for (index, path) in options.allow.iter().enumerate() {
            if path.is_protected()
                || path.as_str().split('/').any(|part| part.starts_with('.'))
                || options.allow[..index]
                    .iter()
                    .any(|prior| selected(path, prior))
            {
                return Err(Error::Arguments);
            }
        }
        let config = Config::load(&options.config).map_err(|_| Error::Paths)?;
        let approved = config
            .canonical_serving_roots(&options.vault, &options.assets, &options.writer_data)
            .map_err(|_| Error::Paths)?;
        let pins = vec![
            Pin::new("canonical", &options.vault, true)?,
            Pin::new("writer_data", &options.writer_data, true)?,
            Pin::new("assets", &options.assets, false)?,
            Pin::new("auth", config.auth_directory(), true)?,
            Pin::new("audit", config.audit_directory(), true)?,
            Pin::new("source", &options.source_snapshot, true)?,
        ];
        let revision = ConfigRevision::read(&options.config)?;
        let mut prepared = Self {
            summary: PlanSummary {
                version: 1,
                plan_digest: String::new(),
                files: 0,
                bytes: 0,
                allow: options.allow.clone(),
            },
            options,
            config,
            approved,
            pins,
            revision,
            notes: Vec::new(),
        };
        prepared.revalidate()?;
        prepared.empty_targets()?;
        prepared.notes = prepared.scan()?;
        prepared.summary.files = prepared.notes.len();
        prepared.summary.bytes = prepared
            .notes
            .iter()
            .map(|note| note.content.len() as u64)
            .sum();
        prepared.summary.plan_digest = prepared.fingerprint(&prepared.notes)?;
        // 独立重读：检测可观察到的漂移，不把这次检查称为文件系统事务快照。
        let repeated = prepared.scan()?;
        if prepared.fingerprint(&repeated)? != prepared.summary.plan_digest {
            return Err(Error::Changed);
        }
        prepared.revalidate()?;
        prepared.empty_targets()?;
        Ok(prepared)
    }

    pub(crate) fn revalidate(&self) -> Result<(), Error> {
        self.approved.revalidate().map_err(|_| Error::Changed)?;
        for pin in &self.pins {
            pin.check()?;
        }
        if ConfigRevision::read(&self.options.config)? != self.revision {
            return Err(Error::Changed);
        }
        let source = self.pins.last().ok_or(Error::Paths)?;
        let identity = (source.record.device, source.record.inode);
        for target in &self.pins[..self.pins.len() - 1] {
            reject_ancestor_identity(
                &source.record.path,
                (target.record.device, target.record.inode),
            )?;
            reject_ancestor_identity(&target.record.path, identity)?;
        }
        reject_ancestor_identity(self.options.config.parent().ok_or(Error::Paths)?, identity)?;
        self.approved.revalidate().map_err(|_| Error::Changed)?;
        Ok(())
    }

    pub(crate) fn empty_targets(&self) -> Result<(), Error> {
        for path in [&self.options.vault, &self.options.writer_data] {
            if fs::read_dir(path)
                .map_err(|_| Error::Paths)?
                .next()
                .is_some()
            {
                return Err(Error::Paths);
            }
        }
        Ok(())
    }

    fn scan(&self) -> Result<Vec<ReadNote>, Error> {
        let source = self.pins.last().ok_or(Error::Source)?;
        source.check()?;
        let batch = source.held.scan(LIMITS).map_err(|_| Error::Source)?;
        if !batch.diagnostics.is_empty()
            || batch.entries.iter().any(|entry| {
                entry.kind == ReadEntryKind::Attachment
                    && self
                        .options
                        .allow
                        .iter()
                        .any(|scope| selected(&entry.path, scope))
            })
            || self.options.allow.iter().any(|scope| {
                !batch
                    .entries
                    .iter()
                    .any(|entry| selected(&entry.path, scope))
            })
        {
            return Err(Error::Source);
        }
        let mut notes: Vec<_> = batch
            .notes
            .into_iter()
            .filter(|note| {
                self.options
                    .allow
                    .iter()
                    .any(|scope| selected(&note.path, scope))
            })
            .collect();
        notes.sort_by(|left, right| left.path.cmp(&right.path));
        if notes
            .iter()
            .any(|note| note.path.is_protected() || std::str::from_utf8(&note.content).is_err())
        {
            return Err(Error::Source);
        }
        source.check()?;
        Ok(notes)
    }

    fn fingerprint(&self, notes: &[ReadNote]) -> Result<String, Error> {
        let input = DigestInput {
            domain: "studiolo-canonical-bootstrap-v1",
            roots: self.pins.iter().map(|pin| &pin.record).collect(),
            config_revision: &self.revision,
            allow: &self.options.allow,
            files: notes
                .iter()
                .map(|note| FileDigest {
                    path: &note.path,
                    bytes: note.content.len(),
                    sha256: digest(&note.content),
                })
                .collect(),
        };
        Ok(digest(
            &serde_json::to_vec(&input).map_err(|_| Error::Source)?,
        ))
    }
}

fn selected(path: &VaultPath, scope: &VaultPath) -> bool {
    path == scope
        || path
            .as_str()
            .strip_prefix(scope.as_str())
            .is_some_and(|tail| tail.starts_with('/'))
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn clean_absolute(path: &Path) -> Result<(), Error> {
    let text = path.to_str().ok_or(Error::Paths)?;
    if !text.starts_with('/')
        || text == "/"
        || text.chars().any(char::is_control)
        || text[1..]
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(Error::Paths);
    }
    Ok(())
}

fn reject_ancestor_identity(path: &Path, forbidden: (u64, u64)) -> Result<(), Error> {
    for parent in path.ancestors() {
        let metadata = fs::symlink_metadata(parent).map_err(|_| Error::Paths)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || (metadata.dev(), metadata.ino()) == forbidden
        {
            return Err(Error::Paths);
        }
    }
    Ok(())
}
