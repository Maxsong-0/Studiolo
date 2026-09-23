//! M6 最终实现的精确调用清单与边界锚点。
use std::path::Path;
pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-runner-adapter" {
        return source.to_owned();
    }
    if path.ends_with("crates/runner-adapter/src/snapshot.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "constRECORD:&str=\".studiolo-snapshot.json\";",
                "constCACHE:&str=\".studiolo-snapshot-cache.json\";",
                "constLOCK:&str=\".studiolo-snapshot.lock\";",
                "constCACHE_SECONDS:u64=60;",
                "letindex=attempt.join(\"snapshot.index\");",
                "letwork=attempt.join(\"work\");",
                "fornamein[\".git\",\".codex\",\".claude\",\".mcp.json\"]",
                "ifpatch.len()>MAX_PATCH",
                ".env_clear().env(\"PATH\",\"/usr/bin:/bin\").env(\"GIT_CONFIG_NOSYSTEM\",\"1\").env(\"GIT_CONFIG_GLOBAL\",\"/dev/null\")",
                "\"--binary\",\"--no-ext-diff\",\"--no-textconv\"",
                "file.try_lock()",
                "let_=self._file.unlock();",
                "ifmeta.dev()!=observed.dev()||meta.ino()!=observed.ino()",
                "if!m.is_file()||m.nlink()!=1||m.uid()!=owner||m.mode()&0o077!=0",
                "request.minimum_age_seconds<300",
                "ifrequest.active_attempts.contains(&attempt)",
                "ifentries.len()>4096",
                "let(metadata,_)=snapshot.preserve(&destination)?;",
                "record.retained=Some(metadata.clone());",
                "write_record(&attempt.join(RECORD),record,lock.owner,true)?;",
                "identity(attempt)?!=record.attempt_identity",
                "identity(&record.admin)?!=record.admin_identity",
            ],
            &[
                (".sync_all()", 4),
                ("fs::{self,File}", 1),
                ("File::open(path)", 4),
                (".write_all(bytes)", 1),
                ("fs::read_dir(dir)", 1),
                (".write_all(&bytes)", 1),
                ("fs::metadata(root)", 1),
                (
                    "Command::new(\"git\").env_clear().env(\"PATH\",\"/usr/bin:/bin\").env(\"GIT_CONFIG_NOSYSTEM\",\"1\").env(\"GIT_CONFIG_GLOBAL\",\"/dev/null\").env(\"GIT_INDEX_FILE\",index).env(\"GIT_TERMINAL_PROMPT\",\"0\").env(\"GIT_AUTHOR_NAME\",\"Studiolosnapshot\").env(\"GIT_AUTHOR_EMAIL\",\"snapshot@invalid\").env(\"GIT_COMMITTER_NAME\",\"Studiolosnapshot\").env(\"GIT_COMMITTER_EMAIL\",\"snapshot@invalid\").arg(\"--git-dir\").arg(repo).arg(\"--work-tree\").arg(work).args([\"-c\",\"core.hooksPath=/dev/null\",\"-c\",\"core.fsmonitor=false\",\"-c\",\"core.untrackedCache=false\",\"-c\",\"diff.external=\",\"-c\",\"core.pager=cat\"]).args(args).stdin(Stdio::null()).output()",
                    1,
                ),
                (
                    "Command::new(\"git\").env_clear().env(\"PATH\",\"/usr/bin:/bin\").env(\"GIT_CONFIG_NOSYSTEM\",\"1\").env(\"GIT_CONFIG_GLOBAL\",\"/dev/null\").args([\"-C\"]).arg(&source).args([\"rev-parse\",\"--absolute-git-dir\"]).output()",
                    1,
                ),
                ("fs::read_dir(&root)", 1),
                ("fs::remove_file(path)", 2),
                ("fs::read(entry.path())", 1),
                ("fs::read_dir(&attempt)", 1),
                ("temporary.persist(path)", 1),
                ("fs::remove_dir_all(path)", 1),
                ("fs::symlink_metadata(path)", 5),
                ("fs::remove_dir_all(attempt)", 1),
                ("std::fs::DirEntry::file_name", 1),
                ("std::os::unix::fs::PermissionsExt", 1),
                ("std::os::unix::fs::MetadataExt", 3),
                ("Result<fs::Metadata,RunnerError>", 2),
                ("fs::Permissions::from_mode(0o700)", 2),
                ("fs::remove_dir_all(&record.admin)", 1),
                ("temporary.persist_noclobber(path)", 2),
                ("fs::read_to_string(work.join(\".git\"))", 1),
                ("tempfile::NamedTempFile::new_in(root)", 1),
                ("tempfile::NamedTempFile::new_in(parent)", 2),
                ("fs::read_to_string(record.admin.join(\"gitdir\"))", 1),
                ("std::os::unix::fs::{MetadataExt,OpenOptionsExt}", 1),
                ("std::os::unix::fs::{MetadataExt,PermissionsExt}", 1),
                (
                    "fs::set_permissions(path,fs::Permissions::from_mode(0o700))",
                    1,
                ),
                (
                    "File::options().read(true).write(true).create(true).truncate(false).mode(0o600).open(&path)",
                    1,
                ),
            ],
        );
    }
    source.to_owned()
}
