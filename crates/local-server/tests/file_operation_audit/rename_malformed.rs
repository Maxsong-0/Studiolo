use super::{
    rename::{rename_committed, rename_intent},
    *,
};

#[test]
fn malformed_rename_never_appends_a_record_or_acknowledges_partial_success() {
    let mut cases = Vec::new();
    let mut value = rename_intent();
    value.schema_version = 1;
    cases.push(value);
    let mut value = rename_intent();
    value.schema_version = 3;
    cases.push(value);
    let mut value = rename_intent();
    if let FileOperationKind::Rename { to, .. } = &mut value.kind {
        *to = value.path.clone();
    }
    cases.push(value);
    let mut value = rename_committed();
    value.effects.swap(0, 1);
    cases.push(value);
    let mut value = rename_committed();
    value.effects.pop();
    cases.push(value);
    let mut value = rename_committed();
    value.effects.push(value.effects[0].clone());
    cases.push(value);
    let mut value = rename_committed();
    value.effects[1] = value.effects[0].clone();
    cases.push(value);
    let mut value = rename_committed();
    value.tombstone_id = None;
    cases.push(value);
    let mut value = rename_committed();
    value.evidence_id = Some("7".repeat(64));
    cases.push(value);
    let mut value = rename_committed();
    if let FileEffect::Confirmed {
        after: FilePresence::Present { revision, .. },
        ..
    } = &mut value.effects[0]
        && let FileOperationKind::Rename { expected, .. } = &value.kind
    {
        *revision = *expected;
    }
    cases.push(value);
    for field in 0..3 {
        let mut value = rename_committed();
        if let FileEffect::Confirmed {
            before:
                FilePresence::Present {
                    revision,
                    sha256,
                    bytes,
                },
            ..
        } = &mut value.effects[1]
        {
            match field {
                0 => *revision = Revision::compute(value.vault_uuid, &value.path, b"other"),
                1 => *sha256 = "8".repeat(64),
                _ => *bytes = 4,
            }
        }
        cases.push(value);
    }
    let mut value = rename_committed();
    if let FileEffect::Confirmed {
        after: FilePresence::Present { bytes, .. },
        ..
    } = &mut value.effects[0]
    {
        *bytes = 268_435_457;
    }
    cases.push(value);
    let mut value = rename_committed();
    value.outcome = Some(FileOperationOutcome::Indeterminate);
    value.tombstone_id = None;
    value.evidence_id = Some("7".repeat(64));
    cases.push(value.clone()); // 整体成功效果不能改标签后冒充未知。
    let FileOperationKind::Rename { to, .. } = &value.kind else {
        unreachable!()
    };
    value.effects = vec![
        FileEffect::Unknown { path: to.clone() },
        FileEffect::Unknown {
            path: value.path.clone(),
        },
    ];
    cases.push(value.clone()); // 不能抹掉已知目标效果。
    value.effects[1] = FileEffect::NotApplied {
        path: value.path.clone(),
    };
    value.evidence_id = None;
    cases.push(value);
    for value in cases {
        let ledger = MemoryAuditLedger::new();
        assert!(append(&ledger, &value).is_err());
        assert!(ledger.query(&AuditFilter::default()).unwrap().is_empty());
    }
}
