use super::*;

#[test]
fn newer_review_and_reopen_both_invalidate_earlier_tickets() {
    let f = Fixture::new();
    let (mut broker, _) = f.interrupted(1);
    let old = broker.pending_mutation().unwrap().unwrap();
    let latest = broker.pending_mutation().unwrap().unwrap();
    assert!(matches!(
        broker.reconcile_pending(&old, PendingDecision::CancelPrepared),
        Err(BrokerError::CaptureChanged)
    ));
    drop(broker);
    let mut reopened = f.open();
    assert!(matches!(
        reopened.reconcile_pending(&latest, PendingDecision::CancelPrepared),
        Err(BrokerError::CaptureChanged)
    ));
    let current = reopened.pending_mutation().unwrap().unwrap();
    reopened
        .reconcile_pending(&current, PendingDecision::CancelPrepared)
        .unwrap();
}

#[test]
fn same_bytes_replacement_or_rename_aba_cannot_use_an_old_review() {
    for replace in [true, false] {
        let f = Fixture::new();
        let (mut broker, _) = f.interrupted(1);
        let review = broker.pending_mutation().unwrap().unwrap();
        let note = f.root.join("note.md");
        let other = f.root.join("other.md");
        if replace {
            fs::write(&other, b"old").unwrap();
            fs::rename(&other, &note).unwrap();
        } else {
            fs::rename(&note, &other).unwrap();
            fs::rename(&other, &note).unwrap();
        }
        assert!(matches!(
            broker.reconcile_pending(&review, PendingDecision::CancelPrepared),
            Err(BrokerError::CaptureChanged)
        ));
        assert!(broker.write_status().unwrap().unresolved);
    }
}

#[test]
fn changed_journal_and_binding_replacement_reject_before_coordination() {
    for damage in ["journal", "marker", "data"] {
        let f = Fixture::new();
        let (mut broker, _) = f.interrupted(1);
        let review = broker.pending_mutation().unwrap().unwrap();
        let changed = match damage {
            "journal" => f.data.join("writer-state.json"),
            "marker" => f.root.join(".workbench/vault.json"),
            _ => f.data.join("broker-binding.json"),
        };
        let replacement = changed.with_extension("replacement");
        fs::write(&replacement, fs::read(&changed).unwrap()).unwrap();
        fs::rename(replacement, changed).unwrap();
        // 日志换了文件身份使审阅所依据的捕获失效；marker 与 data 绑定被换属于配对不符。
        let result = broker.reconcile_pending(&review, PendingDecision::CancelPrepared);
        let rejected = match damage {
            "journal" => matches!(&result, Err(BrokerError::CaptureChanged)),
            _ => matches!(&result, Err(BrokerError::WriterBindingMismatch)),
        };
        assert!(rejected, "{damage}: {:?}", result.err());
        assert_eq!(fs::read(f.root.join("note.md")).unwrap(), b"old");
        assert!(f.data.join("idempotency").read_dir().unwrap().all(|entry| {
            !String::from_utf8_lossy(&fs::read(entry.unwrap().path()).unwrap())
                .contains("cancelled")
        }));
    }
}

#[test]
fn corrupted_stage_or_old_history_does_not_unlock_an_unknown_installation() {
    for damaged in ["stage", "history"] {
        let f = Fixture::new();
        let (mut broker, _) = f.interrupted(2);
        let review = broker.pending_mutation().unwrap().unwrap();
        let file = if damaged == "stage" {
            f.stage(&broker)
        } else {
            f.data
                .join("history")
                .join(review.previous_revision().unwrap().to_string())
        };
        fs::write(file, b"forged").unwrap();
        // 伪造的 stage 与审阅时的捕获不符；伪造的旧历史违反已记录的持久内容证明。
        let result = broker.reconcile_pending(&review, preserve());
        let rejected = match damaged {
            "stage" => matches!(&result, Err(BrokerError::CaptureChanged)),
            _ => matches!(&result, Err(BrokerError::CorruptWriterState)),
        };
        assert!(rejected, "{damaged}: {:?}", result.err());
        assert!(broker.write_status().unwrap().read_blocked);
        assert_eq!(broker.write_status().unwrap().committed_through, 1);
    }
}

#[test]
fn legacy_unknown_and_uncertain_instances_have_no_automatic_recovery_ticket() {
    for legacy in [true, false] {
        let f = Fixture::new();
        let broker = f.initialize();
        if legacy {
            let mut state = crate::mutation::load(&broker).unwrap();
            state.legacy_unresolved = true;
            fs::write(
                f.data.join("writer-state.json"),
                serde_json::to_vec(&state).unwrap(),
            )
            .unwrap();
        } else {
            broker.writer.poison();
        }
        assert!(matches!(
            broker.pending_mutation(),
            Err(BrokerError::MutationUnresolved)
        ));
        assert!(broker.write_status().unwrap().read_blocked);
    }
}

#[test]
fn replay_query_does_not_complete_pending_installation_or_use_current_hash() {
    for point in [1, 2, 3, 4] {
        let f = Fixture::new();
        let (broker, request) = f.interrupted(point);
        let before = fs::read(f.data.join("writer-state.json")).unwrap();
        assert!(broker.mutation_replay(&request).unwrap().is_none());
        assert_eq!(fs::read(f.data.join("writer-state.json")).unwrap(), before);
        assert!(broker.write_status().unwrap().unresolved);
    }
}
