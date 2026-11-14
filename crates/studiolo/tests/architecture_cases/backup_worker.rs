use crate::support::{source_violations, workspace_root};
use std::{fs, path::Path};

#[test]
fn backup_worker_process_capabilities_are_fixed_and_cannot_move_to_another_file() {
    for (file, before, after) in [
        ("process.rs", ".env_clear()", ""),
        (
            "process/child.rs",
            "WaitIdOptions::NOWAIT",
            "WaitIdOptions::empty()",
        ),
        ("process/pipe.rs", "OFlags::NONBLOCK", "OFlags::empty()"),
        ("package.rs", "OFlags::NOFOLLOW", "OFlags::empty()"),
        ("source/parents.rs", "OFlags::NOFOLLOW", "OFlags::empty()"),
        ("credentials/root.rs", "0x8584_58f6", "0"),
        ("credentials/file.rs", "OFlags::NOFOLLOW", "OFlags::empty()"),
        ("config/input.rs", "backup-worker.json", "other.json"),
        ("temporary.rs", "studiolo-backup-worker/tmp", "other/tmp"),
        (
            "credentials.rs",
            "RESTIC_PASSWORD_FILE",
            "OTHER_PASSWORD_FILE",
        ),
    ] {
        let path = std::path::PathBuf::from("crates/backup-worker/src").join(file);
        let source = fs::read_to_string(workspace_root().join(&path)).unwrap();
        assert!(
            source_violations("studiolo-backup-worker", &path, &source).is_empty(),
            "{file}"
        );
        assert!(source.contains(before), "{file}");
        for changed in [
            source.replacen(before, after, 1),
            format!("{source}\nCommand::new(other).env(\"EXTRA\", extra).spawn()?;"),
        ] {
            assert!(
                !source_violations("studiolo-backup-worker", &path, &changed).is_empty(),
                "{file}"
            );
        }
        assert!(
            !source_violations("studiolo-backup-worker", Path::new("src/other.rs"), &source)
                .is_empty(),
            "{file}"
        );
    }
}

#[test]
fn backup_worker_cannot_detach_job_source_or_expiry_from_execution() {
    for (file, before, after) in [
        (
            "source.rs",
            "config.reader_identity()",
            "request.identity()",
        ),
        ("source.rs", "self.binding.check_reopened(&reopened)?;", ""),
        ("runner.rs", "self.source.accepts(invocation)?", "true"),
        ("runner.rs", "&|| self.source.check_time()", "&|| Ok(())"),
        ("source/binding.rs", "invocation == &self.backup", "true"),
        ("source/window.rs", "now < self.last.get()", "false"),
    ] {
        let path = std::path::PathBuf::from("crates/backup-worker/src").join(file);
        let source = fs::read_to_string(workspace_root().join(&path)).unwrap();
        assert!(
            source_violations("studiolo-backup-worker", &path, &source).is_empty(),
            "{file}"
        );
        assert!(source.contains(before), "{file}");
        assert!(
            !source_violations(
                "studiolo-backup-worker",
                &path,
                &source.replace(before, after)
            )
            .is_empty(),
            "{file}"
        );
    }
}
