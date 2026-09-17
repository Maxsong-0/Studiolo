use crate::sqlite::*;
use crate::*;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord};

pub struct Fixture {
    pub root: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir().join(format!("studiolo-t046-bridge-state-draft-{suffix}"));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        // macOS 的系统临时路径可经过 /var 链接；仅夹具选择其真实绝对路径。
        let root = fs::canonicalize(root).unwrap();
        Self { root }
    }
    pub fn init(&self) -> BridgeState {
        BridgeState::initialize(&self.root, binding(), Limits::conservative(1_000_000)).unwrap()
    }
    pub fn reopen(&self) -> BridgeState {
        BridgeState::open_existing(&self.root, binding()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
pub fn binding() -> Binding {
    Binding {
        link_id: [1; 16],
        canonical_vault: [2; 16],
        remote_vault: [3; 16],
        canonical_root_token: [4; 32],
        mirror_root_token: [5; 32],
        scope_hash: [6; 32],
        scope_epoch: 1,
    }
}
pub fn present(n: u8) -> Presence {
    Presence::Present(FileVersion {
        revision: [n; 32],
        sha256: [n; 32],
        bytes: 10,
    })
}
pub fn inventory(
    side: Side,
    sequence: u64,
    entries: BTreeMap<studiolo_core::VaultPath, Presence>,
) -> Inventory {
    Inventory {
        binding: binding(),
        side,
        sequence,
        complete: true,
        entries,
    }
}
pub fn input(store: &BridgeState, count: usize, canonical: Presence, mirror: Presence) -> Input {
    let view = store.view().unwrap();
    let paths = (0..count)
        .map(|i| studiolo_core::VaultPath::parse(format!("项目/记录-{i:04}.md")).unwrap())
        .collect::<Vec<_>>();
    let left = paths
        .iter()
        .map(|p| (p.clone(), canonical.clone()))
        .collect();
    let right = paths.iter().map(|p| (p.clone(), mirror.clone())).collect();
    Input {
        scope: ApprovedScope {
            binding: binding(),
            paths: paths.into_iter().collect(),
        },
        baseline: view.baseline,
        canonical: inventory(
            Side::Canonical,
            view.cursor.last_canonical_observation + 1,
            left,
        ),
        mirror: inventory(Side::Mirror, view.cursor.last_mirror_observation + 1, right),
        expected_baseline_version: view.cursor.baseline_version,
        plan_sequence: view.cursor.last_plan_sequence + 1,
        cursor: view.cursor,
        limits: Limits::conservative(1_000_000),
    }
}
pub fn reserve(store: &mut BridgeState, input: Input) -> Vec<ProgressView> {
    store
        .reserve(store.view().unwrap().version, input.clone())
        .unwrap();
    store.retry_pending(&input).unwrap()
}
pub fn check(store: &BridgeState, input: &Input, action: &Action) -> ExecutionCheck {
    let (source, target) = match action.key.target {
        Side::Canonical => (
            &input.mirror.entries[&action.key.path],
            &input.canonical.entries[&action.key.path],
        ),
        Side::Mirror => (
            &input.canonical.entries[&action.key.path],
            &input.mirror.entries[&action.key.path],
        ),
    };
    ExecutionCheck {
        binding: binding(),
        source: source.clone(),
        target: target.clone(),
        canonical_observation: input.canonical.sequence + 1,
        mirror_observation: input.mirror.sequence + 1,
        process_epoch: store.observation_epoch(),
        expires_at: Instant::now() + Duration::from_secs(30),
    }
}
pub fn common(store: &BridgeState, input: &Input, value: Presence) -> CommonObservation {
    let seq = store.view().unwrap().cursor;
    let entries = input
        .scope
        .paths
        .iter()
        .map(|p| (p.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    CommonObservation {
        canonical: inventory(
            Side::Canonical,
            seq.last_canonical_observation + 1,
            entries.clone(),
        ),
        mirror: inventory(Side::Mirror, seq.last_mirror_observation + 1, entries),
        approved: input.scope.clone(),
        process_epoch: store.observation_epoch(),
        expires_at: Instant::now() + Duration::from_secs(30),
    }
}
pub fn committed(issued: &PendingAction) -> EffectReceipt {
    let (previous, new, hash, bytes) = match &issued.action.change {
        Change::CreateOnly { source } => (
            None,
            Some(source.revision),
            Some(source.sha256),
            Some(source.bytes),
        ),
        Change::Replace {
            source,
            target_expected,
        } => (
            Some(target_expected.revision),
            Some(source.revision),
            Some(source.sha256),
            Some(source.bytes),
        ),
        Change::Delete { target_expected } => (Some(target_expected.revision), None, None, None),
    };
    EffectReceipt {
        external_key: issued.external_key.clone(),
        request_digest: issued.request_digest,
        backend: match issued.action.key.target {
            Side::Canonical => ReceiptBackend::Broker {
                canonical_root: [4; 32],
            },
            Side::Mirror => ReceiptBackend::Mirror {
                mirror_root: [5; 32],
            },
        },
        outcome: ReceiptOutcome::Committed {
            path: issued.action.key.path.as_str().into(),
            previous_revision: previous,
            new_revision: new,
            content_hash: hash,
            bytes,
            receipt_id: "synthetic-adapter-terminal-1".into(),
        },
    }
}
pub fn rejected(issued: &PendingAction) -> EffectReceipt {
    let mut receipt = committed(issued);
    let expected_revision = match &issued.action.change {
        Change::CreateOnly { .. } => None,
        Change::Replace {
            target_expected, ..
        }
        | Change::Delete { target_expected } => Some(target_expected.revision),
    };
    receipt.outcome = ReceiptOutcome::NoEffect {
        path: issued.action.key.path.as_str().into(),
        expected_revision,
        reason: NoEffectReason::CasRejected,
        terminal_id: "synthetic-adapter-rejection-1".into(),
    };
    receipt
}
pub fn flush(store: &mut BridgeState, ledger: &dyn AuditLedger) {
    while store.flush_audit(ledger, 64).unwrap().remaining {}
}
pub struct RejectAudit;
impl AuditLedger for RejectAudit {
    fn append(&self, _: AuditEvent) -> Result<AuditRecord, AuditError> {
        Err(AuditError::Unavailable)
    }
    fn query(&self, _: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        Ok(vec![])
    }
}
