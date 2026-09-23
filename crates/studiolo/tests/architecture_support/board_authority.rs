//! M5/M6 私有权限位置的精确清单；未匹配或重复的调用仍然禁止。
use std::path::Path;
pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-local-server" {
        return source.to_owned();
    }
    if path.ends_with("crates/local-server/src/board/storage.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "letfile=directory.join(\"control.sqlite3\");",
                "f.nlink()!=1",
                "f.permissions().mode()&0o777!=0o600",
                "directory.starts_with(&other)||other.starts_with(directory)",
            ],
            &[
                ("fs::{self,OpenOptions}", 1),
                (
                    "os::unix::fs::{MetadataExtas_,OpenOptionsExtas_,PermissionsExtas_}",
                    1,
                ),
                ("_directory_pin:fs::File,", 1),
                ("_file_pin:fs::File,", 1),
                ("fs::canonicalize(directory)", 1),
                ("fs::canonicalize(other)", 1),
                ("fs::canonicalize(&self.directory)", 1),
                ("fs::symlink_metadata(directory)", 1),
                ("fs::symlink_metadata(&file)", 1),
                ("fs::symlink_metadata(&self.directory)", 1),
                ("fs::symlink_metadata(&self.file)", 1),
                (
                    "OpenOptions::new().write(true).create_new(true).mode(0o600).open(&file)",
                    1,
                ),
                ("fs::File::open(directory)", 1),
                ("fs::File::open(&file)", 1),
            ],
        );
    }
    if path.ends_with("crates/local-server/src/startup/runner_input.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "before.nlink()!=1",
                "before.mode()&0o777!=0o600",
                "directory.mode()&0o777!=0o700",
                "take(131_073)",
                "(before.dev(),before.ino())!=(opened.dev(),opened.ino())",
            ],
            &[
                ("os::unix::fs::MetadataExt", 1),
                ("fs::canonicalize(path)", 1),
                ("fs::symlink_metadata(path)", 2),
                ("fs::symlink_metadata(parent)", 1),
                ("fs::File::open(path)", 1),
            ],
        );
    }
    if path.ends_with("crates/local-server/src/board/executor/config.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "fnprivate_directory(path:&Path)",
                "meta.mode()&0o777!=0o700",
                "meta.uid()!=rustix::process::geteuid().as_raw()",
            ],
            &[
                ("std::os::unix::fs::MetadataExt", 1),
                ("fs::symlink_metadata(path)", 1),
                ("fs::canonicalize(path)", 1),
            ],
        );
    }
    if path.ends_with("crates/local-server/src/board/executor/start.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                ".join(format!(\"{}.capability\",spec.attempt_id))",
                ".attach_runner(",
                "RunnerRef::opaque(profile.provider.name(),&unit)",
            ],
            &[
                ("fs::{File,OpenOptions}", 1),
                ("os::unix::fs::OpenOptionsExt", 1),
                (
                    "OpenOptions::new().write(true).create_new(true).mode(0o600).open(&ticket_path)",
                    1,
                ),
                (
                    "file.write_all(ticket.expose().as_bytes()).and_then(|()|file.sync_all())",
                    1,
                ),
                (
                    "File::open(&self.authority.config.state_root).and_then(|f|f.sync_all())",
                    1,
                ),
            ],
        );
    }
    if path.ends_with("crates/local-server/src/board/executor/publish.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "RelayPolicyGuard::inspect(self.0.as_ref(),grant,now)?;",
                "hash(&bytes)!=digest",
                "metadata.len()>u64::from(grant.limits.response_bytes)",
            ],
            &[("fs::symlink_metadata(raw)", 1), ("fs::read(raw)", 1)],
        );
    }
    if path.ends_with("crates/local-server/src/board/executor/recovery.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "\"--kill-whom=all\",\"--signal=SIGKILL\",unit",
                "s.starts_with('/')&&!s.contains(\"..\")",
                "\"populated0\"",
            ],
            &[
                (
                    "std::fs::read_to_string(format!(\"/sys/fs/cgroup{group}/cgroup.events\"))",
                    1,
                ),
                (
                    "Command::new(\"systemctl\").args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()",
                    1,
                ),
            ],
        );
    }
    if path.ends_with("crates/local-server/src/board/executor/roots.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "fnvalidate_path(path:&Path,forbidden:&[PathBuf])",
                "left.starts_with(right)||right.starts_with(left)",
                ".any(|root|!root.is_absolute()||overlaps(path,root))",
            ],
            &[("std::fs::canonicalize(path)", 1)],
        );
    }
    if path.ends_with("crates/local-server/src/board/executor/lifecycle.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "bindings.remove(&attempt);",
                "self.revoke_ticket(run.spec.attempt_id,&run.ticket)",
                "Err(e)ife.kind()==std::io::ErrorKind::NotFound=>Ok(())",
            ],
            &[
                ("std::fs::remove_file(path)", 1),
                (
                    "std::fs::File::open(&self.authority.config.state_root).and_then(|f|f.sync_all())",
                    1,
                ),
            ],
        );
    }
    source.to_owned()
}
