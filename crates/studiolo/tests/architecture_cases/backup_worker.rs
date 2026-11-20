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
        // 带引号只命中打开处那一个字面量，不命中 CA_PATH。
        (
            "config/input.rs",
            "\"backup-worker-ca.pem\"",
            "\"other.pem\"",
        ),
        (
            "config/input.rs",
            "/etc/studiolo/backup-worker-ca.pem",
            "/etc/other/backup-worker-ca.pem",
        ),
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
            "studiolo_backup::worker::repository_check::CHECK_WINDOW",
            "CHECK_WINDOW",
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
        // 把让路挪回每轮开头：子进程退出、正在读空管道时也会问，查完的结果可能被丢掉。
        (
            "process.rs",
            "        check_source_time()?;\n        let input_progress",
            "        check_source_time()?;\n        interrupt()?;\n        let input_progress",
        ),
        ("process.rs", "if !child.exited()? {", "if true {"),
        ("runtime.rs", "&|| self.window.check(),", "&|| Ok(()),"),
        // T-216：批准的仓库 CA 必须带到每一次连仓库的调用——配置到 Runtime、Runtime 到参数、
        // 三处调用各断一次都判红（`run_check` 那一处原来就有钉子，这里留着作对照）；
        // `runtime.rs` 里多出一处不带 CA 的调用也判红，不管写成直接调用、函数指针、直接引入还是别名。
        (
            "runtime.rs",
            "repository_ca: config.repository_ca(),",
            "repository_ca: None,",
        ),
        (
            "runtime.rs",
            "with_ca(self.repository_ca, argv)",
            "argv.to_vec()",
        ),
        (
            "runtime.rs",
            "let argv = self.arguments(argv);",
            "let argv = argv.to_vec();",
        ),
        (
            "runtime.rs",
            "process::verify_digest(\n            &self.package,\n            &self.arguments(argv),",
            "process::verify_digest(\n            &self.package,\n            argv,",
        ),
        (
            "runtime.rs",
            "process::run_check(\n            &self.package,\n            &self.arguments(argv),",
            "process::run_check(\n            &self.package,\n            argv,",
        ),
        (
            "runtime.rs",
            "    pub fn now(&self)",
            "    pub fn raw(&self, argv: &[String]) -> Result<process::Output> {\n        process::run_scoped(&self.package, argv, None, self.temporary.path(), self.window.deadline(), &|| self.window.check())\n    }\n    pub fn now(&self)",
        ),
        // 函数指针：没有 `process::名字(` 这种写法，照样多出一处函数引用。
        (
            "runtime.rs",
            "    pub fn now(&self)",
            "    pub fn raw(&self) -> usize {\n        let run = process::run_scoped;\n        std::mem::size_of_val(&run)\n    }\n    pub fn now(&self)",
        ),
        // 同一行后面带注释：只截掉注释，这一行前面的函数引用照样数。
        (
            "runtime.rs",
            "    pub fn now(&self)",
            "    pub fn raw(&self) -> usize { std::mem::size_of_val(&process::run_scoped) } // 说明\n    pub fn now(&self)",
        ),
        // 一处类型换成函数引用：`process::` 的出现总数不变，函数引用多一处。
        (
            "runtime.rs",
            "expected: process::DigestExpectation<'_>,",
            "expected: process::run_scoped,",
        ),
        // 直接引入函数名再调用。
        (
            "runtime.rs",
            "use std::{",
            "use crate::process::run_scoped;\nuse std::{",
        ),
        // 花括号引入：函数引用个数不变，`process::` 的出现总数多一次。
        (
            "runtime.rs",
            "use std::{",
            "use crate::process::{run_input};\nuse std::{",
        ),
        // 给模块起别名：两个个数都不变，单独判红。
        (
            "runtime.rs",
            "use std::{",
            "use crate::process as raw;\nuse std::{",
        ),
        // 不连仓库的 `version` 那一处换成别的调用：个数不变，钉子对不上。
        (
            "runtime.rs",
            "&[\"version\".into()],\n            None,",
            "&[\"snapshots\".into()],\n            None,",
        ),
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
    // 对照：注释和文档里提到 `process::名字(` 不算调用点，不判红。
    let path = std::path::PathBuf::from("crates/backup-worker/src/runtime.rs");
    let source = fs::read_to_string(workspace_root().join(&path)).unwrap();
    let before = "    pub fn now(&self)";
    let commented = "    // 见 process::run_check(…)\n    /// 也见 `process::run_scoped(`\n    pub fn now(&self)";
    assert!(source.contains(before));
    let harmless = source.replace(before, commented);
    assert!(source_violations("studiolo-backup-worker", &path, &harmless).is_empty());
    // 抽查窗口只在共用包定一处，最多 20 分钟：放宽到作业的两小时窗口就判红。
    let path = std::path::PathBuf::from("crates/backup/src/worker/repository_check.rs");
    let source = fs::read_to_string(workspace_root().join(&path)).unwrap();
    assert!(source_violations("studiolo-backup", &path, &source).is_empty());
    let before = "Duration::from_secs(20 * 60)";
    assert!(source.contains(before));
    assert!(
        !source_violations(
            "studiolo-backup",
            &path,
            &source.replace(before, "Duration::from_secs(7200)")
        )
        .is_empty()
    );
}
