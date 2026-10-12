//! M6 最终实现的精确调用清单与边界锚点。
use std::path::Path;
pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-runner-adapter" {
        return source.to_owned();
    }
    if path.ends_with("crates/runner-adapter/src/isolation.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "letmutcommand=Command::new(&self.systemd_run);",
                "\"--unshare-net\"",
                "\"--cap-drop\",\"ALL\"",
                "self.runtime_root.join(\"usr/lib/studiolo/rg_probe.py\")",
                "\"--kill-whom=all\"",
            ],
            &[
                ("std::fs::read(probe)", 1),
                (
                    "Command::new(\"systemctl\").args([\"--user\",\"kill\",\"--kill-whom=all\",\"--signal\",signal,unit]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()",
                    1,
                ),
                ("std::os::unix::fs::MetadataExt", 1),
                ("Command::new(&self.systemd_run)", 1),
                ("std::fs::read_link(\"/proc/self/ns/net\")", 1),
            ],
        );
    }
    if path.ends_with("crates/runner-adapter/src/real.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "letscratch=self.config.attempt_root.join(&name);",
                "letinputs=scratch.join(\"inputs\");",
                "include_bytes!(\"../../../deploy/runner/launch.py\")",
                "digest!=self.config.cli_sha256",
                "letpath=run.retained.with_extension(\"stopped.json\");",
                "cancel_marker:inputs.join(\"cancel-requested\")",
                "ifrun.cancelled.is_none()",
                "receipt.get(\"status\").and_then(|v|v.as_str())==Some(\"stopped\")",
            ],
            &[
                (".sync_all()", 2),
                ("fs::read(&path)", 1),
                (".write_all(&bytes)", 1),
                ("fs::read(launcher)", 1),
                ("fs::File::open(binary)", 1),
                ("fs::create_dir(&inputs)", 1),
                ("fs::create_dir(&scratch)", 1),
                ("os::unix::fs::OpenOptionsExt", 1),
                ("fs::Permissions::from_mode(0o700)", 1),
                ("std::os::unix::fs::PermissionsExt", 1),
                ("fs::write(&run.cancel_marker,b\"cancel_requested\")", 1),
                (
                    "fs::set_permissions(&scratch,fs::Permissions::from_mode(0o700))",
                    1,
                ),
                (
                    "fs::write(inputs.join(format!(\"input-{index}\")),&input.content)",
                    1,
                ),
                (
                    "fs::File::open(path.parent().ok_or(RunnerError::InvalidExecution)?)",
                    1,
                ),
                (
                    "fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)",
                    1,
                ),
                (
                    "fs::write(inputs.join(\"job.json\"),serde_json::to_vec(&job).map_err(|_|RunnerError::InvalidExecution)?)",
                    1,
                ),
            ],
        );
    }
    if path.ends_with("crates/runner-adapter/src/landing.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "if!reference.starts_with(\"refs/heads/\")",
                "&[\"update-ref\",reference,new_commit,expected_old]",
                "!=plan.patch_sha256",
                "!=self.expected_head",
                "letindex=directory.path().join(\"index\");",
                "ifbefore!=plan.expected_tree",
                "plan.verify_landed().is_ok()",
            ],
            &[
                ("fs::read(full)", 1),
                ("fs::read(&plan.patch_path)", 1),
                ("fs::Permissions::from_mode(0o700)", 1),
                ("std::os::unix::fs::PermissionsExt", 1),
                ("fs::symlink_metadata(&plan.patch_path)", 1),
            ],
        );
    }
    if path.ends_with("crates/runner-adapter/src/git_trust.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                ".env_clear().env(\"PATH\",\"/usr/bin:/bin\").env(\"GIT_CONFIG_NOSYSTEM\",\"1\").env(\"GIT_CONFIG_GLOBAL\",\"/dev/null\")",
                ".env(\"GIT_NO_LAZY_FETCH\",\"1\")",
                "\"-c\",\"core.hooksPath=/dev/null\"",
                "\"-c\",\"log.showSignature=false\"",
                "\"-c\",\"commit.gpgSign=false\"",
                ".env(\"GIT_COMMITTER_NAME\",\"Studiolorunner\")",
                "constPLAIN_ENTRIES:[&str;10]=",
                "\"refs\",\"logs\",\"worktrees\"]",
                "constPLAIN_TREES:[&str;2]=[\"refs\",\"logs\"];",
                "require_plain_tree(&git_dir.join(name))?;",
                "elseif!meta.is_file()||meta.nlink()!=1",
                "letuid=current_uid()?;",
                "ifgit_dir.parent()!=Some(common_dir.join(\"worktrees\").as_path())",
            ],
            &[
                ("Command::new(\"git\")", 1),
                ("fs::MetadataExt", 1),
                ("fs::canonicalize(&dotgit)", 1),
                ("fs::canonicalize(back)", 1),
                ("fs::canonicalize(git_dir.join(common.trim_end()))", 1),
                ("fs::canonicalize(repo)", 1),
                ("fs::canonicalize(reported)", 1),
                ("fs::canonicalize(target)", 1),
                ("fs::read_dir(&folder)", 1),
                ("fs::read_to_string(&dotgit)", 1),
                ("fs::read_to_string(git_dir.join(\"commondir\"))", 1),
                ("fs::read_to_string(git_dir.join(\"gitdir\"))", 1),
                ("fs::symlink_metadata(&dotgit)", 1),
                ("fs::symlink_metadata(git_dir.join(\"commondir\"))", 1),
                ("fs::symlink_metadata(git_dir.join(name))", 1),
                ("fs::symlink_metadata(path)", 1),
                ("fs::symlink_metadata(root)", 1),
            ],
        );
    }
    if path.ends_with("crates/runner-adapter/src/git_trust/audit.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "constNESTED_LIMIT:usize=64;",
                "key.starts_with(\"hook.\")",
                "ifentry.file_name()==\".git\"",
                "ifaudited>NESTED_LIMIT",
                "audit_config(&nested_git,&folder,true)?;",
                "ifgit_dir!=layout.modules&&git_dir.starts_with(&layout.modules)",
                "elseifgit_dir.parent()==Some(layout.worktrees.as_path())",
                "ifcommon_dir_of(&git_dir)?!=layout.common",
                "!=Some(dotgit.as_path())",
                "iftarget!=work",
            ],
            &[
                ("fs::canonicalize(&dotgit)", 1),
                ("fs::canonicalize(folder.join(target))", 1),
                (
                    "fs::canonicalize(git_dir.join(OsStr::from_bytes(value)))",
                    1,
                ),
                ("fs::canonicalize(git_dir.join(text.trim_end()))", 1),
                ("fs::canonicalize(git_dir.join(back.trim_end()))", 1),
                ("fs::read_dir(&folder)", 1),
                ("fs::read_to_string(&dotgit)", 1),
                ("fs::read_to_string(git_dir.join(\"commondir\"))", 1),
                ("fs::read_to_string(git_dir.join(\"gitdir\"))", 1),
                ("fs::symlink_metadata(&dotgit)", 1),
                ("fs::symlink_metadata(git_dir.join(\"commondir\"))", 1),
            ],
        );
    }
    source.to_owned()
}
