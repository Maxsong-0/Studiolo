use crate::{
    editing::{body, refresh, save},
    fixture::Fixture,
    support,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};
use studiolo_core::VaultUuid;
fn url(f: &Fixture) -> String {
    format!("/api/v1/changes?vault_uuid={}", f.expected.vault_uuid)
}
fn seen(f: &Fixture, page: &Value) -> Value {
    json!({"vault_uuid":f.expected.vault_uuid,"project":page["project"],"snapshot":page["snapshot"],"request_id":VaultUuid::new()})
}
async fn mark(f: &Fixture, issued: &support::Issued, body: &Value, status: u16) -> Value {
    let mut r = f.http.request(
        "POST",
        "/api/v1/changes/seen",
        Some(&issued.cookie),
        &body.to_string(),
    );
    r.headers_mut()
        .insert("x-csrf-token", issued.csrf.parse().unwrap());
    support::json(f.http.send(r, status).await).await
}
#[tokio::test]
async fn changes_preserve_prefetched_rows_fixed_upper_scope_and_cross_device_seen() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    let mut revision = Value::String(f.expected.revision.to_hex());
    for n in 0..8 {
        let mut input = body(
            &f,
            &format!("# 写回 {n}\n"),
            &format!("changes-pagination-{n}"),
        );
        input["expected_revision"] = revision;
        revision = save(&f, &issued, &input, 200).await["receipt"]["new_revision"].clone();
        // 保存后视图要追平才收下一次保存（否则按设计回 503 stale）；不等就成了和后台刷新赛跑。
        refresh(&f, &issued).await;
    }
    fs::create_dir_all(f.http.root.join("vault/项目/Alpha")).unwrap();
    fs::write(f.http.root.join("vault/项目/Alpha/external.md"), "# 外部\n").unwrap();
    refresh(&f, &issued).await;
    let first = f
        .get(&format!("{}&limit=1", url(&f)), &issued.cookie, 200)
        .await;
    assert_eq!(first["through"]["sequence"], "8");
    let initial = first.clone();
    let mut later = body(&f, "# 更晚写回\n", "changes-pagination-later");
    later["expected_revision"] = revision;
    save(&f, &issued, &later, 200).await;
    refresh(&f, &issued).await;
    let mut page = first;
    let mut ids = BTreeSet::new();
    let mut broker = 0;
    for _ in 0..30 {
        for row in page["entries"].as_array().unwrap() {
            assert!(ids.insert(row["id"].as_str().unwrap().to_owned()));
            if row["source"] == "broker" {
                broker += 1;
                assert!(row["sequence"].as_str().unwrap().parse::<u64>().unwrap() <= 8);
                assert_eq!(row["actor"]["kind"], "human");
            }
        }
        let Some(next) = page["next_cursor"].as_str() else {
            break;
        };
        let endpoint = format!("{}&limit=1&before={next}", url(&f));
        let current = f.get(&endpoint, &issued.cookie, 200).await;
        let replay = f.get(&endpoint, &issued.cookie, 200).await;
        assert_eq!(current["entries"], replay["entries"]);
        assert_eq!(current["through"], initial["through"]);
        page = current;
    }
    assert!(page["next_cursor"].is_null());
    assert_eq!(broker, 8);
    let request = seen(&f, &initial);
    let marked = mark(&f, &issued, &request, 200).await;
    assert_eq!(mark(&f, &issued, &request, 200).await, marked);
    let other = f.http.issue().await;
    let current = f.get(&url(&f), &other.cookie, 200).await;
    let entries = current["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .filter(|v| v["source"] == "broker")
            .all(|v| v["unread"] == (v["sequence"] == "9"))
    );
    let project_url = format!("{}&project=%E9%A1%B9%E7%9B%AE%2FAlpha", url(&f));
    let project = f.get(&project_url, &other.cookie, 200).await;
    assert!(
        project["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| !v["path"].is_null())
            .all(|v| v["path"].as_str().unwrap().starts_with("项目/Alpha/") && v["unread"] == true)
    );
    let mut changed = request.clone();
    changed["project"] = json!("项目/Alpha");
    assert_eq!(
        mark(&f, &issued, &changed, 409).await["code"],
        "seen_conflict"
    );
    mark(&f, &other, &seen(&f, &project), 200).await;
    let read = f.get(&project_url, &issued.cookie, 200).await;
    assert!(
        read["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["unread"] == false)
    );
    f.close().await;
}
#[derive(Default)]
struct SwitchAudit {
    ledger: MemoryAuditLedger,
    mode: AtomicU8,
}
impl AuditLedger for SwitchAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let mode = self.mode.load(Ordering::Acquire);
        if mode == 1
            || (mode == 2
                && event.action() == "content.changes_seen"
                && event.phase() == AuditPhase::Outcome)
        {
            return Err(AuditError::Unavailable);
        }
        self.ledger.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        if self.mode.load(Ordering::Acquire) == 1 {
            return Err(AuditError::Unavailable);
        }
        self.ledger.query(filter)
    }
}
#[tokio::test]
async fn changes_seen_requires_l2_csrf_and_exact_snapshot_and_recovers_pending_outcome() {
    let audit = Arc::new(SwitchAudit::default());
    let f = Fixture::with_audit(audit.clone()).await;
    let issued = f.http.issue().await;
    f.http
        .send(f.http.request("GET", &url(&f), None, ""), 401)
        .await;
    let page = f.get("/api/v1/changes", &issued.cookie, 200).await;
    assert_eq!(page["vault_uuid"], f.expected.vault_uuid.to_string());
    let input = seen(&f, &page);
    f.http
        .send(
            f.http.request(
                "POST",
                "/api/v1/changes/seen",
                Some(&issued.cookie),
                &input.to_string(),
            ),
            403,
        )
        .await;
    let mut future = input.clone();
    future["sequence"] = json!(999999);
    mark(&f, &issued, &future, 400).await;
    let mut forged = input.clone();
    forged["snapshot"] = json!(VaultUuid::new());
    mark(&f, &issued, &forged, 409).await;
    let mut wrong = input.clone();
    wrong["project"] = json!("项目/Alpha");
    mark(&f, &issued, &wrong, 409).await;
    audit.mode.store(2, Ordering::Release);
    mark(&f, &issued, &input, 503).await;
    let pending = f.get(&url(&f), &issued.cookie, 200).await;
    assert!(
        pending["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["unread"] == true)
    );
    audit.mode.store(0, Ordering::Release);
    mark(&f, &issued, &input, 200).await;
    assert_eq!(
        audit
            .ledger
            .query(&AuditFilter::default().with_action("content.changes_seen"))
            .unwrap()
            .len(),
        2
    );
    audit.mode.store(1, Ordering::Release);
    let outage = f.get(&url(&f), &issued.cookie, 200).await;
    assert_eq!(outage["seen_complete"], false);
    assert!(
        outage["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["unread"].is_null())
    );
    mark(&f, &issued, &input, 503).await;
    audit.mode.store(0, Ordering::Release);
    f.close().await;
}
