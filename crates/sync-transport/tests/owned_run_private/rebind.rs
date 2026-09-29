//! 重绑定纯状态与会话契约：不重置历史，不复活旧静止能力，失败不发布。

use super::support::*;
use crate::owned_run::rebind::transition;
use crate::owned_run::*;

fn next(fixture: &Fixture) -> Result<TransportState> {
    let runtime = PinnedRuntime::from_approval(&fixture.approval)?;
    transition(&fixture.slot.state(), &fixture.approval, &runtime, OWNER)
}

fn open(fixture: &Fixture) -> Result<TransportRebind<'_>> {
    TransportRebind::open(&fixture.approval, fixture.slot.publisher())
}

fn close(fixture: &Fixture) -> QuiescentMirror {
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).unwrap();
    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = false;
    session.wait(&owned).unwrap();
    acknowledge(&mut session);
    session.quiescent().unwrap()
}

#[test]
fn runtime_backend_budget_and_same_inode_path_changes_preserve_installation_and_sequence() {
    let changes: [fn(&mut FakeApproval); 6] = [
        |a| a.runtime.closure_digest = [9; 32],
        |a| a.backend.borrow_mut().digest = [8; 32],
        |a| a.budget.borrow_mut().deadline_seconds = 123,
        |a| a.roots.borrow_mut().private_root = "/srv/renamed-private".into(),
        |a| a.roots.borrow_mut().mirror_root = "/srv/renamed-payload".into(),
        |a| a.roots.borrow_mut().mirror_view_root = "/srv/renamed-view".into(),
    ];
    for change in changes {
        let mut fixture = Fixture::new();
        close(&fixture);
        let old = fixture.slot.state();
        change(&mut fixture.approval);
        fixture.approval.binding.borrow_mut().approved_config_digest = [7; 32];
        let changed = next(&fixture).unwrap();
        assert_eq!(changed.installation_id, old.installation_id);
        assert_eq!(changed.next_sequence, old.next_sequence);
        assert_eq!(changed.generation, old.generation + 1);
        assert_eq!(changed.roots, fixture.approval.roots());
        assert_eq!(changed.runtime, fixture.approval.runtime());
        assert_eq!(changed.backend, fixture.approval.backend());
        assert!(changed.active.is_none() && changed.last_terminal.is_none());
        assert_eq!(fixture.slot.state(), old, "纯变换不发布");
        open(&fixture).unwrap().commit().unwrap();
        assert_eq!(fixture.slot.state(), changed);
    }
}

#[test]
fn scope_vault_link_remote_hash_or_epoch_drift_is_not_a_transport_rebind() {
    let changes: [fn(&mut TransportBindingFacts); 5] = [
        |b| b.vault_uuid = studiolo_core::VaultUuid::new(),
        |b| b.link_id = [7; 16],
        |b| b.remote_vault = [7; 16],
        |b| b.scope_hash = [7; 32],
        |b| b.scope_epoch = std::num::NonZeroU64::new(2).unwrap(),
    ];
    for change in changes {
        let fixture = Fixture::new();
        let old = fixture.slot.bytes();
        change(&mut fixture.approval.binding.borrow_mut());
        assert_eq!(open(&fixture).err(), Some(TransportError::BindingMismatch));
        assert_eq!(fixture.slot.bytes(), old);
    }
}

#[test]
fn physical_private_or_mirror_replacement_is_never_a_path_rename() {
    for private in [true, false] {
        let fixture = Fixture::new();
        let old = fixture.slot.bytes();
        if private {
            fixture.approval.roots.borrow_mut().private_identity.1 += 1;
        } else {
            fixture.approval.roots.borrow_mut().mirror_identity.1 += 1;
        }
        assert_eq!(open(&fixture).err(), Some(TransportError::RootsChanged));
        assert_eq!(fixture.slot.bytes(), old);
    }
}

#[test]
fn prepared_pending_intent_or_running_slot_cannot_be_discarded_by_rebind() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = session.prepare_once(1, RunOperation::RunOnce, 1).unwrap();
    let pending = fixture.slot.bytes();
    assert_eq!(open(&fixture).err(), Some(TransportError::AuditPending));
    assert_eq!(fixture.slot.bytes(), pending);
    acknowledge(&mut session);
    assert_eq!(open(&fixture).err(), Some(TransportError::RunActive));
    let owned = session.start(&prepared).unwrap();
    assert_eq!(open(&fixture).err(), Some(TransportError::RunActive));
    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = false;
    session.wait(&owned).unwrap();
    assert_eq!(open(&fixture).err(), Some(TransportError::AuditPending));
}

#[test]
fn an_unconfirmed_or_nonclosed_terminal_is_not_a_static_mirror_proof() {
    for unconfirmed in [true, false] {
        let fixture = Fixture::new();
        close(&fixture);
        let mut state = fixture.slot.state();
        let terminal = state.last_terminal.as_mut().unwrap();
        if unconfirmed {
            terminal.observation.quiescence = Quiescence::Unconfirmed;
        } else {
            terminal.observation.phase = RunPhase::OutcomePending;
        }
        fixture.slot.set_bytes(state::encode(&state).unwrap());
        assert_eq!(
            open(&fixture).err(),
            Some(TransportError::QuiescenceUnconfirmed)
        );
    }
}

#[test]
fn generation_overflow_and_owner_mismatch_are_refused_without_publication() {
    let fixture = Fixture::new();
    let mut state = fixture.slot.state();
    state.generation = u64::MAX;
    fixture.slot.set_bytes(state::encode(&state).unwrap());
    assert_eq!(open(&fixture).err(), Some(TransportError::CounterOverflow));
    state.generation = 1;
    state.owner.uid += 1;
    fixture.slot.set_bytes(state::encode(&state).unwrap());
    assert_eq!(open(&fixture).err(), Some(TransportError::UnsafeStorage));
}

#[test]
fn repeated_safe_calls_and_reopening_cannot_reissue_or_revive_old_quiescent_tickets() {
    let fixture = Fixture::new();
    let ticket = close(&fixture);
    let original = fixture.slot.state();
    for ordinal in 1..=2 {
        let review = open(&fixture).unwrap().commit().unwrap();
        assert_eq!(review.next_generation, original.generation + ordinal);
        let reopened = fixture.session();
        assert_eq!(reopened.quiescent().err(), Some(TransportError::NotFound));
        assert_eq!(
            reopened.verify_quiescent(&ticket),
            Err(TransportError::TicketForeign)
        );
        assert_eq!(
            fixture.slot.state().installation_id,
            original.installation_id
        );
        assert_eq!(fixture.slot.state().next_sequence, original.next_sequence);
    }
}

#[test]
fn publication_failure_keeps_old_bytes_and_a_retry_starts_from_that_state() {
    let fixture = Fixture::new();
    let old = fixture.slot.bytes();
    fixture.slot.fail_next_replace(TransportError::Unavailable);
    assert_eq!(
        open(&fixture).unwrap().commit(),
        Err(TransportError::Unavailable)
    );
    assert_eq!(fixture.slot.bytes(), old);
    open(&fixture).unwrap().commit().unwrap();
    assert_eq!(fixture.slot.state().generation, 2);
}

#[test]
fn changed_approval_or_store_after_review_is_refused_before_commit() {
    let fixture = Fixture::new();
    let pending = open(&fixture).unwrap();
    let old = fixture.slot.bytes();
    fixture.approval.binding.borrow_mut().approved_config_digest = [9; 32];
    assert_eq!(pending.commit(), Err(TransportError::ConfigurationChanged));
    assert_eq!(fixture.slot.bytes(), old);
    let pending = open(&fixture).unwrap();
    let mut state = fixture.slot.state();
    state.generation += 1;
    fixture.slot.set_bytes(state::encode(&state).unwrap());
    assert_eq!(pending.commit(), Err(TransportError::GenerationConflict));
    let pending = open(&fixture).unwrap();
    *fixture.approval.fresh.borrow_mut() = false;
    assert_eq!(pending.commit(), Err(TransportError::ConfigurationChanged));
}

#[test]
fn old_software_need_not_exist_and_new_runtime_must_match_the_approval() {
    let mut fixture = Fixture::new();
    let mut state = fixture.slot.state();
    state.runtime.node_path = "/removed/old-node".into();
    state.runtime.cli_path = "/removed/old-cli.js".into();
    fixture.slot.set_bytes(state::encode(&state).unwrap());
    open(&fixture).unwrap().commit().unwrap();
    fixture.approval.runtime.version = "unsupported".to_owned();
    assert_eq!(open(&fixture).err(), Some(TransportError::RuntimeChanged));
}
