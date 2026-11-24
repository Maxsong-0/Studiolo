use crate::support::{source_violations, workspace_root};
use std::{fs, path::Path};
#[test]
fn producer_requires_persistent_admission_original_intent_and_sealed_audit_tail() {
    for (crate_name, file, anchor) in [
        (
            "studiolo-local-server",
            "crates/local-server/src/backup/audit.rs",
            "self.admission.verify()?;",
        ),
        (
            "studiolo-local-server",
            "crates/local-server/src/backup/admission.rs",
            "verify_source(&ledger, &record)?;",
        ),
        (
            "studiolo-backup",
            "crates/backup/src/worker/audit/admission.rs",
            "now >= self.expires_at",
        ),
        (
            "studiolo-backup",
            "crates/backup/src/worker/audit.rs",
            "fact.key() == self.key()",
        ),
    ] {
        let path = Path::new(file);
        let source = fs::read_to_string(workspace_root().join(path)).unwrap();
        assert!(
            source_violations(crate_name, path, &source).is_empty(),
            "{file}"
        );
        assert!(source.contains(anchor), "{file}");
        assert!(
            !source_violations(crate_name, path, &source.replacen(anchor, "removed", 1)).is_empty(),
            "{file}"
        );
    }
}
#[test]
fn backup_audit_socket_identity_and_native_connection_are_closed_capabilities() {
    for owner in [
        "SqliteAuditLedger",
        "MemoryAuditLedger",
        "AuditReadView",
        "AuditCheckpointAccess",
        "AuditCheckpointSource",
        "FrozenAudit",
    ] {
        assert!(
            !source_violations(
                "studiolo-backup-worker",
                Path::new("src/other.rs"),
                &format!("use studiolo_audit_ledger::{owner};")
            )
            .is_empty()
        );
    }
    for (file, anchor) in [
        ("audit.rs", "filter.limit() != Some(1)"),
        ("audit/endpoint.rs", "self.check(deadline)?;"),
        ("audit/endpoint/proof.rs", "OFlags::NOFOLLOW"),
    ] {
        let path = std::path::PathBuf::from("crates/backup-worker/src").join(file);
        let source = fs::read_to_string(workspace_root().join(&path)).unwrap();
        assert!(
            source_violations("studiolo-backup-worker", &path, &source).is_empty(),
            "{file}"
        );
        assert!(source.contains(anchor), "{file}");
        for changed in [
            source.replacen(anchor, "removed", 1),
            format!("{source}\nnet::connect(other, other_address)?;"),
            format!("{source}\nstd::fs::write(other, bytes)?;"),
        ] {
            assert!(
                !source_violations("studiolo-backup-worker", &path, &changed).is_empty(),
                "{file}"
            );
        }
        if file != "audit.rs" {
            assert!(
                !source_violations("studiolo-backup-worker", Path::new("src/other.rs"), &source)
                    .is_empty()
            );
        }
    }
    let path = Path::new("crates/backup/src/worker/audit/transport.rs");
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations("studiolo-backup", path, &source).is_empty());
    for anchor in [
        "!= self.peer_uid",
        "self.poisoned = true;",
        "self.read_some(&mut extra)? != 0",
    ] {
        assert!(source.contains(anchor));
        assert!(
            !source_violations(
                "studiolo-backup",
                path,
                &source.replacen(anchor, "removed", 1)
            )
            .is_empty()
        );
    }
}
#[test]
fn producer_socket_and_schedule_are_closed_capabilities() {
    let crate_name = "studiolo-local-server";
    let path = Path::new("crates/local-server/src/backup/listener.rs");
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations(crate_name, path, &source).is_empty());
    // 父目录 0750、socket 0660、已有节点拒绝、单一名字：和 worker 一侧的核对一一对应。
    for anchor in [
        "0o750",
        "0o660",
        "Err(Errno::NOENT)",
        "stat.st_nlink != 1",
        "identity(&stat) == self.identity",
    ] {
        assert!(source.contains(anchor), "{anchor}");
        assert!(
            !source_violations(crate_name, path, &source.replacen(anchor, "removed", 1)).is_empty(),
            "{anchor}"
        );
    }
    for changed in [
        format!("{source}\nUnixListener::bind(other)?;"),
        format!("{source}\nstd::fs::write(other, bytes)?;"),
        format!("{source}\nlet _ = unlinkat(&other, name, AtFlags::empty());"),
        format!("{source}\nchmodat(&other, name, Mode::from_raw_mode(0o666), AtFlags::empty())?;"),
    ] {
        assert!(!source_violations(crate_name, path, &changed).is_empty());
    }
    // 换一个文件名就拿不到放行。
    assert!(
        !source_violations(
            crate_name,
            Path::new("crates/local-server/src/backup/other.rs"),
            &source
        )
        .is_empty()
    );

    let path = Path::new("crates/local-server/src/backup/producer.rs");
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations(crate_name, path, &source).is_empty());
    for anchor in [
        "self.plan.worker_uid == rustix::process::geteuid().as_raw()",
        "AuditSocket::new(stream, self.plan.worker_uid, deadline)",
        "request.target() != TARGET",
        "BackupAdmission::resume(self.ledger.clone(), request.key(), TARGET)",
        "queue.outstanding()",
        "self.schedule_with(now_millis)",
        "if !due(now, interval, queue.last_attempt_ms())",
        // `self.window.close();` 在别处还有一处：连同前一句一起认，删掉放开之后那一处才算数。
        "let mut released = frozen\n            .release_timed(deadline, &mut steps)\n            .map_err(|error| self.stuck(&error, &steps))?;\n        self.window.close();",
        "drop(released);",
        "QueueCommand::RepositoryCheck => self.repository_check()?",
        "return self.report_check(&request);",
    ] {
        assert!(source.contains(anchor), "{anchor}");
        assert!(
            !source_violations(crate_name, path, &source.replacen(anchor, "removed", 1)).is_empty(),
            "{anchor}"
        );
    }

    // 抽查只在空闲时派；结果要有同一检查键、本目标的唯一原意图才落账。
    let path = Path::new("crates/local-server/src/backup/producer/check.rs");
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations(crate_name, path, &source).is_empty());
    for anchor in [
        "if state.busy {",
        "if state.queue.outstanding() {",
        "if checks.running(now).is_some() {",
        "if record.event() != &expected {",
        "if assignment.key == *request.key() && target == TARGET",
        "if !same {",
        "if records.len() > 1 {",
    ] {
        assert!(source.contains(anchor), "{anchor}");
        assert!(
            !source_violations(crate_name, path, &source.replacen(anchor, "if false {", 1))
                .is_empty(),
            "{anchor}"
        );
    }
    // 挂满时限的原意图由生产者补「中途没了」，不能删掉这一步；生产入口不能换成假时钟。
    for (before, after) in [
        (
            "for key in &abandoned {",
            "for key in &Vec::<CheckKey>::new() {",
        ),
        ("check_abandoned(key)", "check_intent(&assignment, TARGET)"),
        (
            "self.repository_check_with(now_millis)",
            "self.repository_check_with(|| Ok(0))",
        ),
    ] {
        assert!(source.contains(before), "{before}");
        assert!(
            !source_violations(crate_name, path, &source.replacen(before, after, 1)).is_empty(),
            "{before}"
        );
    }
}
