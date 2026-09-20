//! 仅允许可信 root 操作者在固定演练副本和独立网络命名空间内装配。
//! 备份来源/清单由外层包装器证明；这里不把目录布局冒充来源证明。

use crate::{AdminError, options::identifier};
use rustix::fs::{Mode, OFlags, open, openat};
use std::{
    fs::{self, File, Metadata},
    io::{self, IsTerminal},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

pub(crate) const BASE: &str = "/var/lib/studiolo-dr-tests";
const FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK)
    .union(OFlags::CLOEXEC);

#[derive(Clone, Copy)]
enum Kind {
    Ancestor,
    Directory,
    Database,
    Config,
}

struct Pin {
    path: PathBuf,
    file: File,
    before: Metadata,
    kind: Kind,
}

pub(crate) struct Guard {
    pins: Vec<Pin>,
    root: PathBuf,
    namespaces: [File; 2],
}

pub(crate) fn layout(root: &Path, config: &Path) -> Result<(), AdminError> {
    let value = root.to_str().ok_or(AdminError::RestoreGate)?;
    let tail = value
        .strip_prefix(&format!("{BASE}/"))
        .ok_or(AdminError::RestoreGate)?;
    let (run, leaf) = tail.split_once('/').ok_or(AdminError::RestoreGate)?;
    if !identifier(run, 64)
        || leaf != "human-work"
        || config.as_os_str() != root.join("owner.json").as_os_str()
    {
        return Err(AdminError::RestoreGate);
    }
    Ok(())
}

fn context(
    linux: bool,
    real: u32,
    effective: u32,
    tty: bool,
    separate: bool,
) -> Result<(), AdminError> {
    if !linux || real != 0 || effective != 0 || tty || !separate {
        return Err(AdminError::RestoreGate);
    }
    Ok(())
}

fn identity(metadata: &Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

fn permissions(mode: u32, owner: u32, links: u64, kind: Kind) -> bool {
    owner == 0
        && match kind {
            // 固定 /var/lib 布局没有临时目录祖先，不接受 sticky 放宽。
            Kind::Ancestor => links > 0 && mode & 0o022 == 0,
            Kind::Directory => links > 0 && mode & 0o7777 == 0o700,
            Kind::Database | Kind::Config => links == 1 && mode & 0o7777 == 0o600,
        }
}

fn inspect(metadata: &Metadata, kind: Kind, device: u64) -> Result<(), AdminError> {
    let directory = matches!(kind, Kind::Ancestor | Kind::Directory);
    if metadata.dev() != device
        || metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
        || !permissions(metadata.mode(), metadata.uid(), metadata.nlink(), kind)
    {
        return Err(AdminError::RestoreGate);
    }
    Ok(())
}

impl Pin {
    fn revalidate(&self) -> Result<(), AdminError> {
        for current in [self.file.metadata(), fs::symlink_metadata(&self.path)] {
            let current = current.map_err(|_| AdminError::RestoreGate)?;
            inspect(&current, self.kind, self.before.dev())?;
            if identity(&current) != identity(&self.before) || current.gid() != self.before.gid() {
                return Err(AdminError::RestoreGate);
            }
            if matches!(self.kind, Kind::Config)
                && (
                    current.len(),
                    current.mtime(),
                    current.mtime_nsec(),
                    current.ctime(),
                    current.ctime_nsec(),
                ) != (
                    self.before.len(),
                    self.before.mtime(),
                    self.before.mtime_nsec(),
                    self.before.ctime(),
                    self.before.ctime_nsec(),
                )
            {
                return Err(AdminError::RestoreGate);
            }
        }
        Ok(())
    }
}

impl Guard {
    pub(crate) fn pin(root: &Path, config: &Path) -> Result<Self, AdminError> {
        context(
            cfg!(target_os = "linux"),
            rustix::process::getuid().as_raw(),
            rustix::process::geteuid().as_raw(),
            io::stdin().is_terminal(),
            true,
        )?;
        let namespaces = [
            File::open("/proc/self/ns/net"),
            File::open("/proc/1/ns/net"),
        ]
        .map(|result| result.map_err(|_| AdminError::RestoreGate));
        let [own, initial] = namespaces;
        let namespaces = [own?, initial?];
        let own = namespaces[0]
            .metadata()
            .map_err(|_| AdminError::RestoreGate)?;
        let initial = namespaces[1]
            .metadata()
            .map_err(|_| AdminError::RestoreGate)?;
        context(true, 0, 0, false, identity(&own) != identity(&initial))?;
        layout(root, config)?;
        let file = File::from(
            open("/", FLAGS | OFlags::DIRECTORY, Mode::empty())
                .map_err(|_| AdminError::RestoreGate)?,
        );
        let before = file.metadata().map_err(|_| AdminError::RestoreGate)?;
        inspect(&before, Kind::Ancestor, before.dev())?;
        let mut guard = Self {
            pins: vec![Pin {
                path: "/".into(),
                file,
                before,
                kind: Kind::Ancestor,
            }],
            root: root.into(),
            namespaces,
        };
        let mut parent = 0;
        for name in root.to_str().ok_or(AdminError::RestoreGate)?[1..].split('/') {
            let private = guard.pins[parent].path.join(name).starts_with(BASE);
            parent = guard.child(
                parent,
                name,
                if private {
                    Kind::Directory
                } else {
                    Kind::Ancestor
                },
            )?;
        }
        guard.child(parent, "owner.json", Kind::Config)?;
        for directory in ["auth", "audit"] {
            let child = guard.child(parent, directory, Kind::Directory)?;
            guard.child(child, &format!("{directory}.sqlite3"), Kind::Database)?;
        }
        guard.revalidate()?;
        Ok(guard)
    }

    fn child(&mut self, parent: usize, name: &str, kind: Kind) -> Result<usize, AdminError> {
        let flags = FLAGS
            | if matches!(kind, Kind::Ancestor | Kind::Directory) {
                OFlags::DIRECTORY
            } else {
                OFlags::empty()
            };
        let file = File::from(
            openat(&self.pins[parent].file, name, flags, Mode::empty())
                .map_err(|_| AdminError::RestoreGate)?,
        );
        let before = file.metadata().map_err(|_| AdminError::RestoreGate)?;
        inspect(&before, kind, self.pins[parent].before.dev())?;
        if matches!(kind, Kind::Config | Kind::Database) && before.len() == 0 {
            return Err(AdminError::RestoreGate);
        }
        let pin = Pin {
            path: self.pins[parent].path.join(name),
            file,
            before,
            kind,
        };
        pin.revalidate()?;
        self.pins.push(pin);
        Ok(self.pins.len() - 1)
    }

    pub(crate) fn revalidate(&self) -> Result<(), AdminError> {
        for pin in &self.pins {
            pin.revalidate()?;
        }
        let current = File::open("/proc/self/ns/net").map_err(|_| AdminError::RestoreGate)?;
        if identity(&current.metadata().map_err(|_| AdminError::RestoreGate)?)
            != identity(
                &self.namespaces[0]
                    .metadata()
                    .map_err(|_| AdminError::RestoreGate)?,
            )
        {
            return Err(AdminError::RestoreGate);
        }
        let entries = |path: &Path| -> Result<Vec<_>, AdminError> {
            fs::read_dir(path)
                .map_err(|_| AdminError::RestoreGate)?
                .map(|entry| entry.map_err(|_| AdminError::RestoreGate))
                .collect()
        };
        let mut names = entries(&self.root)?
            .into_iter()
            .map(|entry| entry.file_name())
            .collect::<Vec<_>>();
        names.sort();
        if names != ["audit", "auth", "owner.json"].map(std::ffi::OsString::from) {
            return Err(AdminError::RestoreGate);
        }
        for directory in ["auth", "audit"] {
            let base = self.root.join(directory);
            let pinned = self
                .pins
                .iter()
                .find(|pin| pin.path == base)
                .ok_or(AdminError::RestoreGate)?;
            for entry in entries(&base)? {
                let name = entry.file_name();
                if !["", "-wal", "-shm", "-journal"]
                    .iter()
                    .any(|suffix| name == format!("{directory}.sqlite3{suffix}").as_str())
                {
                    return Err(AdminError::RestoreGate);
                }
                let file = File::from(
                    openat(&pinned.file, &name, FLAGS, Mode::empty())
                        .map_err(|_| AdminError::RestoreGate)?,
                );
                let metadata = file.metadata().map_err(|_| AdminError::RestoreGate)?;
                inspect(&metadata, Kind::Database, pinned.before.dev())?;
                if identity(&metadata)
                    != identity(
                        &fs::symlink_metadata(entry.path()).map_err(|_| AdminError::RestoreGate)?,
                    )
                {
                    return Err(AdminError::RestoreGate);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/support/restore_guard_tests.rs"]
mod tests;
