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
        let source = fs::read_to_string(workspace_root().join(&path)).unwrap();
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
