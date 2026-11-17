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
        ("process/input.rs", "OFlags::NONBLOCK", "OFlags::empty()"),
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
        ("job.rs", "audit.admitted_source()", "request.fact()"),
        ("job.rs", "fact: AdmissionFact,", "pub fact: AdmissionFact,"),
        ("runner.rs", "job.check_live()?;", ""),
        ("runner.rs", ".verify_sealed(source.sealed())", ".skip()"),
        ("runner/final_record.rs", ".verify_draft(draft)", ".skip()"),
        (
            "runner/final_record.rs",
            "draft.source_root() != self.source_root",
            "false",
        ),
        (
            "source.rs",
            "config.reader_identity()",
            "request.identity()",
        ),
        ("source.rs", "self.binding.check_reopened(&reopened)?;", ""),
        ("runner.rs", "self.source.accepts(invocation)?", "true"),
        (
            "runner.rs",
            "let source_time = || self.source.check_time();",
            "let source_time = || Ok(());",
        ),
        ("source/binding.rs", "invocation == &self.backup", "true"),
        ("source/window.rs", "now < self.last.get()", "false"),
        (
            "runtime.rs",
            "self.poisoned.set(true)",
            "self.poisoned.set(false)",
        ),
        ("runner/final_record.rs", ".borrow_mut()", ".borrow()"),
        (
            "runner/final_record/read.rs",
            "sha256: &hash",
            "sha256: request.digest()",
        ),
        (
            "runner/final_record/plan.rs",
            "\"--stdin\".into()",
            "\"--stdin-from-command\".into()",
        ),
        ("runner/inspect.rs", "file.sha256()", "request.digest()"),
        (
            "runner/probe.rs",
            "self.verify_snapshot_files(snapshot)?;",
            "",
        ),
        (
            "runner/inspect/plan.rs",
            "\"dump\".into()",
            "\"forget\".into()",
        ),
        // 抽查：清理没能证实必须退出；最多 20 分钟；遇到待办作业要让路；作业优先于抽查。
        (
            "repository_check.rs",
            "ProcessError::CleanupFailed => return Checked::Shutdown,",
            "ProcessError::CleanupFailed => IncompleteReason::Unavailable,",
        ),
        (
            "repository_check.rs",
            "Duration::from_secs(20 * 60)",
            "Duration::from_secs(7200)",
        ),
        (
            "repository_check.rs",
            "CredentialHolder::BackupWorker",
            "CredentialHolder::App",
        ),
        (
            "serve.rs",
            "Ok(Some(_)) => Err(ProcessError::Interrupted),",
            "Ok(Some(_)) => Ok(()),",
        ),
        (
            "serve.rs",
            "Ok(None) => return idle(residency, config, held),",
            "Ok(None) => return Ok(Round::Waiting),",
        ),
        ("process.rs", "interrupt()?;", ""),
        ("runtime.rs", "&|| self.window.check(),", "&|| Ok(()),"),
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
