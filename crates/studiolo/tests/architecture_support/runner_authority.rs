//! Exact call inventories and boundary anchors for the final M6 implementation.
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
                ".env_clear().env(\"PATH\",\"/usr/bin:/bin\").env(\"GIT_CONFIG_NOSYSTEM\",\"1\").env(\"GIT_CONFIG_GLOBAL\",\"/dev/null\")",
                "letindex=directory.path().join(\"index\");",
                "ifbefore!=plan.expected_tree",
                "plan.verify_landed().is_ok()",
            ],
            &[
                ("fs::read(full)", 1),
                ("Command::new(\"git\")", 1),
                ("fs::read(&plan.patch_path)", 1),
                ("fs::Permissions::from_mode(0o700)", 1),
                ("std::os::unix::fs::PermissionsExt", 1),
                ("fs::symlink_metadata(&plan.patch_path)", 1),
            ],
        );
    }
    source.to_owned()
}
