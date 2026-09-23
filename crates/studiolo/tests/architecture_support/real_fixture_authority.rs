//! M6 最终实现的精确调用清单与边界锚点。
use std::path::Path;
pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-runner-adapter" {
        return source.to_owned();
    }
    if path.ends_with("crates/runner-adapter/src/real_tests.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "lettemp=tempfile::tempdir().unwrap();",
                "letroot=temp.path().canonicalize().unwrap();",
                "forpathin[&source,&attempts,&attempt,&retained]",
                ".env_clear().args([\"-c\",&format!(\"exit{exit}\")])",
                "fncancellation_is_idempotent_and_never_infers_a_stopped_receipt()",
            ],
            &[
                ("fs::read(&path)", 1),
                ("fs::metadata(path)", 1),
                (
                    "Command::new(\"git\").env_clear().env(\"PATH\",\"/usr/bin:/bin\").env(\"GIT_CONFIG_GLOBAL\",\"/dev/null\").env(\"GIT_CONFIG_NOSYSTEM\",\"1\").env(\"GIT_AUTHOR_NAME\",\"fixture\").env(\"GIT_AUTHOR_EMAIL\",\"fixture@invalid\").env(\"GIT_COMMITTER_NAME\",\"fixture\").env(\"GIT_COMMITTER_EMAIL\",\"fixture@invalid\").arg(\"-C\").arg(path).args(args).output()",
                    1,
                ),
                ("fs::create_dir(path)", 1),
                (
                    "Command::new(\"/bin/sh\").env_clear().args([\"-c\",&format!(\"exit{exit}\")]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()",
                    1,
                ),
                ("fs::create_dir(&inputs)", 1),
                ("os::unix::fs::PermissionsExt", 1),
                ("fs::Permissions::from_mode(0o700)", 1),
                ("fs::read(&runs[&fixture.handle].cancel_marker)", 1),
                ("fs::write(source.join(\"file.txt\"),\"baseline\\n\")", 1),
                (
                    "fs::write(snapshot.work.join(\"file.txt\"),\"candidate\\n\")",
                    1,
                ),
                (
                    "fs::set_permissions(path,fs::Permissions::from_mode(0o700))",
                    1,
                ),
            ],
        );
    }
    source.to_owned()
}
