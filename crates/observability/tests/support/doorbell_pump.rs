//! 门铃主循环对内存账本、内存状态和假发送器：顺序、审计先行、退避、过期、停用、崩溃后重跑。

use super::*;
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};
use studiolo_audit_ledger::{AuditRecord, MemoryAuditLedger, OperationPermit};

const MINUTE: i64 = 60 * 1000;

struct Store {
    state: Mutex<Option<DoorbellState>>,
    load_error: Mutex<Option<StoreError>>,
    refuse_saves: AtomicBool,
}

impl Store {
    fn new() -> Self {
        Self {
            state: Mutex::new(None),
            load_error: Mutex::new(None),
            refuse_saves: AtomicBool::new(false),
        }
    }

    fn state(&self) -> DoorbellState {
        self.state.lock().unwrap().clone().expect("应当已有状态")
    }
}

impl StateStore for Store {
    fn load(&self) -> Result<Option<DoorbellState>, StoreError> {
        match *self.load_error.lock().unwrap() {
            Some(error) => Err(error),
            None => Ok(self.state.lock().unwrap().clone()),
        }
    }

    fn save(&self, state: &DoorbellState) -> Result<(), StoreError> {
        if self.refuse_saves.load(Ordering::SeqCst) {
            return Err(StoreError::Unavailable);
        }
        *self.state.lock().unwrap() = Some(state.clone());
        Ok(())
    }
}

struct Posted {
    message_id: String,
    timestamp_secs: i64,
    body: serde_json::Value,
}

struct Transport {
    tag: Mutex<Result<String, PostError>>,
    link: Option<String>,
    replies: Mutex<VecDeque<Result<(), PostError>>>,
    posted: Mutex<Vec<Posted>>,
}

impl Transport {
    fn new() -> Self {
        Self {
            tag: Mutex::new(Ok("00000000000000aa".into())),
            link: None,
            replies: Mutex::new(VecDeque::new()),
            posted: Mutex::new(Vec::new()),
        }
    }

    /// 接下来几次投递的回执；用完之后一律收下。
    fn reply(&self, replies: &[Result<(), PostError>]) {
        self.replies.lock().unwrap().extend(replies.iter().copied());
    }

    fn ids(&self) -> Vec<String> {
        self.posted
            .lock()
            .unwrap()
            .iter()
            .map(|posted| posted.message_id.clone())
            .collect()
    }
}

impl DoorbellTransport for Transport {
    fn endpoint_tag(&self) -> Result<String, PostError> {
        self.tag.lock().unwrap().clone()
    }

    fn link(&self) -> Option<&str> {
        self.link.as_deref()
    }

    fn post(&self, message_id: &str, timestamp_secs: i64, body: &[u8]) -> Result<(), PostError> {
        self.posted.lock().unwrap().push(Posted {
            message_id: message_id.into(),
            timestamp_secs,
            body: serde_json::from_slice(body).expect("事件正文是 JSON"),
        });
        self.replies.lock().unwrap().pop_front().unwrap_or(Ok(()))
    }
}

/// 可以让追加、记结果或取操作许可失败的账本，用来演「审计不可用」和各个崩溃点。
struct Ledger {
    inner: MemoryAuditLedger,
    refuse_appends: AtomicBool,
    /// 只拒绝门铃自己的结果记录：演「发出去了、结果没记上」。
    refuse_outcomes: AtomicBool,
    refuse_operations: AtomicBool,
    operations: AtomicU32,
}

impl Ledger {
    fn new() -> Self {
        Self {
            inner: MemoryAuditLedger::new(),
            refuse_appends: AtomicBool::new(false),
            refuse_outcomes: AtomicBool::new(false),
            refuse_operations: AtomicBool::new(false),
            operations: AtomicU32::new(0),
        }
    }

    fn add(&self, id: &str, action: &str, payload: serde_json::Value) -> AuditRecord {
        self.inner
            .append(AuditEvent::new(
                AuditEventId::parse(id).unwrap(),
                AuditPhase::Outcome,
                action,
                SanitizedPayload::try_from(payload).unwrap(),
            ))
            .unwrap()
    }

    fn task(&self, n: u32) -> AuditRecord {
        self.add(
            &format!("board:task.create:{n}"),
            "task.create",
            serde_json::json!({"request_hash": n.to_string(), "status": "succeeded"}),
        )
    }

    fn noise(&self, n: u32) -> AuditRecord {
        self.add(
            &format!("read-{n}"),
            "content.read",
            serde_json::json!({"tool": "read_note"}),
        )
    }

    /// 账本里门铃自己记下的（阶段、结论）序列。
    fn dispatches(&self) -> Vec<(AuditPhase, String)> {
        self.inner
            .query(&AuditFilter::default().with_action(DOORBELL_AUDIT_ACTION))
            .unwrap()
            .iter()
            .map(|record| {
                let payload = record.event().payload().as_value();
                (
                    record.event().phase(),
                    payload["classification"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }
}

impl AuditLedger for Ledger {
    fn enter_operation(&self) -> Result<OperationPermit, AuditError> {
        if self.refuse_operations.load(Ordering::SeqCst) {
            return Err(AuditError::Unavailable);
        }
        self.operations.fetch_add(1, Ordering::SeqCst);
        self.inner.enter_operation()
    }

    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let doorbell_outcome =
            event.action() == DOORBELL_AUDIT_ACTION && event.phase() == AuditPhase::Outcome;
        if self.refuse_appends.load(Ordering::SeqCst)
            || (doorbell_outcome && self.refuse_outcomes.load(Ordering::SeqCst))
        {
            return Err(AuditError::Unavailable);
        }
        // 门铃的每一次追加都必须发生在它自己取到的操作许可之内。
        if event.action() == DOORBELL_AUDIT_ACTION || event.action() == DOORBELL_ENABLED_ACTION {
            assert!(
                self.operations.load(Ordering::SeqCst) > 0,
                "门铃没取操作许可就写了审计"
            );
        }
        self.inner.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}

fn now() -> i64 {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("时钟在 1970 年之后");
    i64::try_from(elapsed.as_millis()).expect("毫秒时间戳装得下")
}

/// 已经启用过的一套：账本、状态、发送器。
fn enabled() -> (Ledger, Store, Transport) {
    let (ledger, store, transport) = (Ledger::new(), Store::new(), Transport::new());
    let summary = run(&ledger, &store, &transport, now()).unwrap();
    assert_eq!(summary.outcome, RunOutcome::Enabled);
    (ledger, store, transport)
}

fn caught_up(delivered: u32, expired: u32) -> RunSummary {
    RunSummary {
        outcome: RunOutcome::CaughtUp,
        delivered,
        expired,
    }
}

#[test]
fn enabling_is_audited_and_never_replays_earlier_history() {
    let (ledger, store, transport) = (Ledger::new(), Store::new(), Transport::new());
    ledger.task(1);
    let summary = run(&ledger, &store, &transport, now()).unwrap();
    assert_eq!(summary.outcome, RunOutcome::Enabled);
    let enabled = ledger
        .inner
        .query(&AuditFilter::default().with_action(DOORBELL_ENABLED_ACTION))
        .unwrap();
    assert_eq!(enabled.len(), 1);
    assert_eq!(store.state().cursor, enabled[0].offset());
    // 启用之前的那条任务不补发；之后新来的才发。
    assert_eq!(
        run(&ledger, &store, &transport, now()).unwrap(),
        caught_up(0, 0)
    );
    assert!(transport.ids().is_empty());
    ledger.task(2);
    assert_eq!(
        run(&ledger, &store, &transport, now()).unwrap(),
        caught_up(1, 0)
    );
    assert_eq!(transport.ids().len(), 1);
}

#[test]
fn whitelisted_events_leave_in_ledger_order_and_everything_else_stays() {
    let (ledger, store, mut transport) = enabled();
    transport.link = Some("https://studiolo.example.test/".into());
    ledger.noise(1);
    let first = classify(&ledger.task(1)).unwrap();
    ledger.noise(2);
    let second = classify(&ledger.task(2)).unwrap();
    let last = ledger.noise(3);
    let at = now();
    assert_eq!(
        run(&ledger, &store, &transport, at).unwrap(),
        caught_up(2, 0)
    );
    assert_eq!(transport.ids(), [first.message_id(), second.message_id()]);
    for posted in transport.posted.lock().unwrap().iter() {
        assert_eq!(posted.timestamp_secs, at / 1000);
        assert_eq!(posted.body["type"], "task.created");
        assert_eq!(
            posted.body["data"],
            serde_json::json!({"link": "https://studiolo.example.test/"})
        );
    }
    let delivered = || (AuditPhase::Outcome, "delivered".to_owned());
    let intent = || (AuditPhase::Intent, "intent".to_owned());
    assert_eq!(
        ledger.dispatches(),
        [intent(), delivered(), intent(), delivered()]
    );
    // 位置停在门铃自己最后一条审计之后也没关系；至少越过了最后一条无关记录。
    assert!(store.state().cursor >= last.offset());
    // 再跑一次：门铃自己的审计不在白名单里，不会自己叫醒自己。
    assert_eq!(
        run(&ledger, &store, &transport, at).unwrap(),
        caught_up(0, 0)
    );
    assert_eq!(transport.ids().len(), 2);
}

#[test]
fn a_failed_head_event_blocks_the_rest_and_backs_off_on_schedule() {
    let (ledger, store, transport) = enabled();
    let first = classify(&ledger.task(1)).unwrap();
    ledger.task(2);
    let at = now();
    transport.reply(&[Err(PostError::Unavailable), Err(PostError::Timeout)]);

    let summary = run(&ledger, &store, &transport, at).unwrap();
    assert_eq!(summary.outcome, RunOutcome::Deferred("unavailable"));
    assert_eq!((summary.delivered, transport.ids().len()), (0, 1));
    // 第一次失败后马上可以再试（下一次定时器）。
    assert_eq!((store.state().attempts, store.state().retry_at_ms), (1, at));

    let summary = run(&ledger, &store, &transport, at).unwrap();
    assert_eq!(summary.outcome, RunOutcome::Deferred("timeout"));
    assert_eq!(store.state().attempts, 2);
    assert_eq!(store.state().retry_at_ms, at + 5 * MINUTE);

    // 退避期内不碰网络。
    let summary = run(&ledger, &store, &transport, at + MINUTE).unwrap();
    assert_eq!(summary.outcome, RunOutcome::Backoff);
    assert_eq!(transport.ids().len(), 2);

    // 到点后队首先送到，后面那条才跟着出门；重发用的是同一个消息编号。
    let later = at + 5 * MINUTE;
    assert_eq!(
        run(&ledger, &store, &transport, later).unwrap(),
        caught_up(2, 0)
    );
    let ids = transport.ids();
    assert_eq!(ids.len(), 4);
    assert!(ids[..3].iter().all(|id| id == first.message_id()));
    assert_ne!(ids[3], first.message_id());
    assert_eq!((store.state().attempts, store.state().retry_at_ms), (0, 0));
    let outcomes: Vec<String> = ledger
        .dispatches()
        .into_iter()
        .filter(|(phase, _)| *phase == AuditPhase::Outcome)
        .map(|(_, classification)| classification)
        .collect();
    assert_eq!(
        outcomes,
        ["unavailable", "timeout", "delivered", "delivered"]
    );
}

#[test]
fn the_backoff_table_is_capped() {
    let minutes: Vec<i64> = (1..=8)
        .map(|failures| backoff_ms(failures) / MINUTE)
        .collect();
    assert_eq!(minutes, [0, 5, 30, 120, 300, 600, 600, 600]);
    assert_eq!(backoff_ms(u32::MAX), 600 * MINUTE);
}

#[test]
fn a_gone_endpoint_stays_silent_until_the_address_changes() {
    let (ledger, store, transport) = enabled();
    ledger.task(1);
    transport.reply(&[Err(PostError::Gone)]);
    let at = now();
    let summary = run(&ledger, &store, &transport, at).unwrap();
    assert_eq!(summary.outcome, RunOutcome::EndpointGone);
    let held = store.state().cursor;
    for _ in 0..3 {
        let summary = run(&ledger, &store, &transport, at + 24 * 60 * MINUTE).unwrap();
        assert_eq!(summary.outcome, RunOutcome::EndpointGone);
    }
    assert_eq!(transport.ids().len(), 1);
    // 换了收件地址：从原来的位置接着发，那条没送到的事件不丢。
    *transport.tag.lock().unwrap() = Ok("00000000000000bb".into());
    assert_eq!(
        run(&ledger, &store, &transport, at).unwrap(),
        caught_up(1, 0)
    );
    assert!(store.state().cursor > held && !store.state().gone);
    assert_eq!(store.state().endpoint_tag, "00000000000000bb");
}

#[test]
fn events_older_than_a_day_are_skipped_with_an_audit_record() {
    let (ledger, store, transport) = enabled();
    ledger.task(1);
    ledger.task(2);
    let late = now() + MAX_EVENT_AGE_MS + MINUTE;
    assert_eq!(
        run(&ledger, &store, &transport, late).unwrap(),
        caught_up(0, 2)
    );
    assert!(transport.ids().is_empty());
    let expired = (AuditPhase::Outcome, "expired".to_owned());
    assert_eq!(ledger.dispatches(), [expired.clone(), expired]);
    assert_eq!(
        run(&ledger, &store, &transport, late).unwrap(),
        caught_up(0, 0)
    );
}

#[test]
fn nothing_is_sent_when_the_intent_cannot_be_audited() {
    let (ledger, store, transport) = enabled();
    ledger.task(1);
    let before = store.state();
    ledger.refuse_appends.store(true, Ordering::SeqCst);
    assert_eq!(
        run(&ledger, &store, &transport, now()),
        Err(RunError::AuditUnavailable)
    );
    assert!(transport.ids().is_empty());
    assert_eq!(store.state(), before);
    ledger.refuse_appends.store(false, Ordering::SeqCst);
    assert_eq!(
        run(&ledger, &store, &transport, now()).unwrap(),
        caught_up(1, 0)
    );
}

#[test]
fn a_crash_after_delivery_does_not_send_the_same_event_again() {
    let (ledger, store, transport) = enabled();
    ledger.task(1);
    // 送到了、审计也记了，只是状态没来得及落盘就退出。
    store.refuse_saves.store(true, Ordering::SeqCst);
    assert_eq!(
        run(&ledger, &store, &transport, now()),
        Err(RunError::StateUnavailable)
    );
    assert_eq!(transport.ids().len(), 1);
    store.refuse_saves.store(false, Ordering::SeqCst);
    // 重跑：账本里已有「送到」的结果，只把位置补上，不再发第二遍，也不算进这次送到的条数。
    assert_eq!(
        run(&ledger, &store, &transport, now()).unwrap(),
        caught_up(0, 0)
    );
    assert_eq!(transport.ids().len(), 1);
    assert_eq!(
        run(&ledger, &store, &transport, now()).unwrap(),
        caught_up(0, 0)
    );
}

#[test]
fn a_crash_after_a_failed_attempt_retries_under_a_new_round() {
    let (ledger, store, transport) = enabled();
    ledger.task(1);
    transport.reply(&[Err(PostError::Unavailable)]);
    store.refuse_saves.store(true, Ordering::SeqCst);
    assert_eq!(
        run(&ledger, &store, &transport, now()),
        Err(RunError::StateUnavailable)
    );
    store.refuse_saves.store(false, Ordering::SeqCst);
    // 状态里的失败次数没记上；重跑不能去改写第 0 轮已有的失败记录。
    assert_eq!(store.state().attempts, 0);
    assert_eq!(
        run(&ledger, &store, &transport, now()).unwrap(),
        caught_up(1, 0)
    );
    let rounds: Vec<String> = ledger
        .inner
        .query(&AuditFilter::default().with_action(DOORBELL_AUDIT_ACTION))
        .unwrap()
        .iter()
        .map(|record| {
            let payload = record.event().payload().as_value();
            format!(
                "{}:{}",
                payload["round"].as_str().unwrap(),
                payload["classification"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(
        rounds,
        ["0:intent", "0:unavailable", "1:intent", "1:delivered"]
    );
}

#[test]
fn one_run_delivers_a_bounded_batch_and_the_next_run_continues() {
    let (ledger, store, transport) = enabled();
    for n in 0..40 {
        ledger.task(n);
    }
    assert_eq!(
        run(&ledger, &store, &transport, now()).unwrap(),
        caught_up(32, 0)
    );
    assert_eq!(
        run(&ledger, &store, &transport, now()).unwrap(),
        caught_up(8, 0)
    );
    let ids = transport.ids();
    assert_eq!(ids.len(), 40);
    let unique: std::collections::BTreeSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), 40);
}

#[test]
fn a_damaged_state_or_missing_configuration_fails_closed() {
    let (ledger, store, transport) = enabled();
    ledger.task(1);
    *store.load_error.lock().unwrap() = Some(StoreError::Corrupt);
    // 状态坏了不当成「还没启用」：不重新记启用位置，也不发。
    assert_eq!(
        run(&ledger, &store, &transport, now()),
        Err(RunError::StateUnavailable)
    );
    assert!(transport.ids().is_empty());
    *store.load_error.lock().unwrap() = None;

    for (configured, expected) in [
        (PostError::NotConfigured, RunError::NotConfigured),
        (PostError::Malformed, RunError::Malformed),
    ] {
        *transport.tag.lock().unwrap() = Err(configured);
        assert_eq!(run(&ledger, &store, &transport, now()), Err(expected));
        let fresh = Store::new();
        assert_eq!(run(&ledger, &fresh, &transport, now()), Err(expected));
        assert!(fresh.state.lock().unwrap().is_none());
    }
    assert!(transport.ids().is_empty());
}

#[test]
fn a_crash_between_enabling_and_saving_does_not_skip_what_came_after() {
    let (ledger, store, transport) = (Ledger::new(), Store::new(), Transport::new());
    // 启用记进了审计，状态没存上就退出。
    store.refuse_saves.store(true, Ordering::SeqCst);
    assert_eq!(
        run(&ledger, &store, &transport, now()),
        Err(RunError::StateUnavailable)
    );
    store.refuse_saves.store(false, Ordering::SeqCst);
    // 两次之间来了一件事：它排在启用记录之后，不能被一个更晚的起点跳过去。
    ledger.task(1);
    let summary = run(&ledger, &store, &transport, now()).unwrap();
    assert_eq!(summary.outcome, RunOutcome::Enabled);
    let enabled = ledger
        .inner
        .query(&AuditFilter::default().with_action(DOORBELL_ENABLED_ACTION))
        .unwrap();
    assert_eq!(enabled.len(), 1, "重试拿回的是原来那条启用记录");
    assert_eq!(store.state().cursor, enabled[0].offset());
    assert_eq!(
        run(&ledger, &store, &transport, now()).unwrap(),
        caught_up(1, 0)
    );
    assert_eq!(transport.ids().len(), 1);
}

#[test]
fn losing_the_state_file_rescans_from_the_start_without_resending_or_skipping() {
    let (ledger, store, transport) = enabled();
    ledger.task(1);
    ledger.task(2);
    let at = now();
    assert_eq!(
        run(&ledger, &store, &transport, at).unwrap(),
        caught_up(2, 0)
    );
    // 一条过期跳过的，再加一条还没来得及发的。
    ledger.task(3);
    let late = at + MAX_EVENT_AGE_MS + MINUTE;
    assert_eq!(
        run(&ledger, &store, &transport, late).unwrap(),
        caught_up(0, 1)
    );
    let recorded = ledger.dispatches().len();
    let pending = ledger.task(4);

    *store.state.lock().unwrap() = None;
    // 第 4 条是按真实时钟记进账本的，重扫用真实的「现在」，它才不算过期。
    let summary = run(&ledger, &store, &transport, at).unwrap();
    assert_eq!(summary.outcome, RunOutcome::Enabled);
    // 回到原起点重扫：送到过的两条、过期的一条都不再发、不再记；只有第 4 条出门。
    assert_eq!(
        run(&ledger, &store, &transport, at).unwrap(),
        caught_up(1, 0)
    );
    let ids = transport.ids();
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[2], classify(&pending).unwrap().message_id());
    assert_eq!(ledger.dispatches().len(), recorded + 2);
}

#[test]
fn every_event_is_handled_inside_its_own_operation_permit() {
    let (ledger, store, transport) = enabled();
    assert_eq!(ledger.operations.load(Ordering::SeqCst), 1, "启用占一张");
    ledger.noise(1);
    ledger.task(1);
    ledger.task(2);
    let before = store.state();

    // 备份封存占着、取不到许可：一条都不发，意图也不记，位置不动。
    ledger.refuse_operations.store(true, Ordering::SeqCst);
    assert_eq!(
        run(&ledger, &store, &transport, now()),
        Err(RunError::AuditUnavailable)
    );
    assert!(transport.ids().is_empty() && ledger.dispatches().is_empty());
    assert_eq!(store.state().attempts, before.attempts);

    ledger.refuse_operations.store(false, Ordering::SeqCst);
    assert_eq!(
        run(&ledger, &store, &transport, now()).unwrap(),
        caught_up(2, 0)
    );
    // 两条白名单事件各一张；无关记录不占许可。
    assert_eq!(ledger.operations.load(Ordering::SeqCst), 3);
}

#[test]
fn a_gone_reply_is_saved_before_its_outcome_is_recorded() {
    let (ledger, store, transport) = enabled();
    ledger.task(1);
    transport.reply(&[Err(PostError::Gone)]);
    // 收到 410 之后、结果还没记进审计就出了事。
    ledger.refuse_outcomes.store(true, Ordering::SeqCst);
    assert_eq!(
        run(&ledger, &store, &transport, now()),
        Err(RunError::AuditUnavailable)
    );
    ledger.refuse_outcomes.store(false, Ordering::SeqCst);
    assert!(store.state().gone, "停用必须先于结果落盘");
    // 重跑不会换一个轮次再去敲已经说不要了的门。
    for _ in 0..2 {
        let summary = run(&ledger, &store, &transport, now()).unwrap();
        assert_eq!(summary.outcome, RunOutcome::EndpointGone);
    }
    assert_eq!(transport.ids().len(), 1);
    // 换地址后那条事件照常送出去：意图沿用原轮次，没有改写已有记录。
    *transport.tag.lock().unwrap() = Ok("00000000000000bb".into());
    assert_eq!(
        run(&ledger, &store, &transport, now()).unwrap(),
        caught_up(1, 0)
    );
    assert_eq!(transport.ids().len(), 2);
}
