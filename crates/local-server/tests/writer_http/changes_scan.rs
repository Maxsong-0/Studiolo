//! 变更列表第一页在 owner 锁和观测日志锁外读已读水位（T-251）：扫描被卡住时，别的请求照常完成；
//! 扫描期间提交的写入、发布的观察不混进这一页，也不让它的续页失效。
use crate::{
    editing::{body, refresh, save},
    fixture::Fixture,
    support,
};
use serde_json::{Value, json};
use std::{
    fs,
    future::Future,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord, MemoryAuditLedger,
};
use studiolo_core::VaultUuid;
use tokio::sync::Notify;

/// 每个并发请求最多等这么久；改回持锁扫描的话，它们要一直等到扫描放行，这里先超时判红。
const BOUND: Duration = Duration::from_secs(10);

/// 只卡已读水位扫描的第一下（定位账本尾从偏移 0 之后取 1 条，`seen::tail`），放行前一直停着；别的查询、追加照常。
#[derive(Default)]
struct GateAudit {
    ledger: MemoryAuditLedger,
    armed: AtomicBool,
    entered: Notify,
    released: (Mutex<bool>, Condvar),
}
impl GateAudit {
    fn arm(&self) {
        *self.released.0.lock().unwrap() = false;
        self.armed.store(true, Ordering::Release);
    }
    fn release(&self) {
        *self.released.0.lock().unwrap() = true;
        self.released.1.notify_all();
    }
}
impl AuditLedger for GateAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        self.ledger.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        let tail_probe = filter.event_id().is_none()
            && filter.offset_exclusive() == Some(0)
            && filter.limit() == Some(1);
        if tail_probe && self.armed.swap(false, Ordering::AcqRel) {
            self.entered.notify_one();
            let (released, signal) = &self.released;
            let mut open = released.lock().unwrap();
            while !*open {
                open = signal.wait(open).unwrap();
            }
        }
        self.ledger.query(filter)
    }
}

/// 扫描卡着时做的事必须在 `BOUND` 内做完；超时先放行扫描（不然卡住的线程收不走），再判红。
async fn bounded<T>(audit: &GateAudit, what: &str, work: impl Future<Output = T>) -> T {
    match tokio::time::timeout(BOUND, work).await {
        Ok(value) => value,
        Err(_) => {
            audit.release();
            panic!("{what}在第一页读已读水位时被挡住，说明扫描还占着 owner 锁或观测日志锁");
        }
    }
}

fn url(f: &Fixture) -> String {
    format!("/api/v1/changes?vault_uuid={}", f.expected.vault_uuid)
}
async fn mark(f: &Fixture, issued: &support::Issued, page: &Value) -> Value {
    let input = json!({"vault_uuid":f.expected.vault_uuid,"project":page["project"],"snapshot":page["snapshot"],"request_id":VaultUuid::new()});
    let mut r = f.http.request(
        "POST",
        "/api/v1/changes/seen",
        Some(&issued.cookie),
        &input.to_string(),
    );
    r.headers_mut()
        .insert("x-csrf-token", issued.csrf.parse().unwrap());
    support::json(f.http.send(r, 200).await).await
}
fn broker_rows(page: &Value) -> Vec<&Value> {
    page["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["source"] == "broker")
        .collect()
}
async fn wait_scanning(audit: &GateAudit) {
    if tokio::time::timeout(BOUND, audit.entered.notified())
        .await
        .is_err()
    {
        audit.release();
        panic!("第一页没有去读已读水位");
    }
}

#[tokio::test]
async fn first_page_scans_seen_watermark_outside_owner_and_observation_locks() {
    let audit = Arc::new(GateAudit::default());
    let f = Fixture::with_audit(audit.clone()).await;
    let issued = f.http.issue().await;
    let saved = save(
        &f,
        &issued,
        &body(&f, "# 第一版\n", "changes-scan-first-save"),
        200,
    )
    .await;
    refresh(&f, &issued).await;
    let list = url(&f);
    let earlier = f.get(&list, &issued.cookie, 200).await;
    audit.arm();
    let gated = f.get(&list, &issued.cookie, 200);
    let during = async {
        wait_scanning(&audit).await;
        // 只问水位要拿 owner 锁和观测日志锁；标已读、保存、刷新视图都要 owner 锁。
        let head = bounded(
            &audit,
            "只问水位",
            f.get(
                &format!("/api/v1/changes/head?vault_uuid={}", f.expected.vault_uuid),
                &issued.cookie,
                200,
            ),
        )
        .await;
        bounded(&audit, "标已读", mark(&f, &issued, &earlier)).await;
        let mut next = body(&f, "# 扫描期间写回\n", "changes-scan-second-save");
        next["expected_revision"] = saved["receipt"]["new_revision"].clone();
        bounded(&audit, "保存", save(&f, &issued, &next, 200)).await;
        bounded(&audit, "刷新视图", refresh(&f, &issued)).await;
        audit.release();
        head
    };
    let (page, head) = tokio::join!(gated, during);
    assert_eq!(head["through"], earlier["through"]);
    assert_eq!(page["through"], earlier["through"]);
    assert_eq!(page["seen_complete"], true);
    f.close().await;
}

#[tokio::test]
async fn writes_and_seen_marks_during_the_watermark_scan_neither_leak_into_nor_expire_the_first_page()
 {
    let audit = Arc::new(GateAudit::default());
    let f = Fixture::with_audit(audit.clone()).await;
    let issued = f.http.issue().await;
    let other = f.http.issue().await;
    let mut revision = Value::String(f.expected.revision.to_hex());
    for n in 0..3 {
        let mut input = body(
            &f,
            &format!("# 写回 {n}\n"),
            &format!("changes-scan-pre-{n}"),
        );
        input["expected_revision"] = revision;
        revision = save(&f, &issued, &input, 200).await["receipt"]["new_revision"].clone();
        refresh(&f, &issued).await;
    }
    fs::create_dir_all(f.http.root.join("vault/项目/Alpha")).unwrap();
    fs::write(f.http.root.join("vault/项目/Alpha/before.md"), "# 之前\n").unwrap();
    refresh(&f, &issued).await;
    let earlier = f.get(&url(&f), &issued.cookie, 200).await;
    assert!(
        broker_rows(&earlier)
            .iter()
            .all(|row| row["unread"] == true)
    );
    let limited = format!("{}&limit=1", url(&f));
    audit.arm();
    let gated = f.get(&limited, &issued.cookie, 200);
    let during = async {
        wait_scanning(&audit).await;
        // 另一台设备把扫描开始前那份快照标成已读；同一时间又有一次 Broker 写入和一处外部新文件被发布成观察。
        bounded(&audit, "另一台设备标已读", mark(&f, &other, &earlier)).await;
        let mut input = body(&f, "# 扫描期间写回\n", "changes-scan-during");
        input["expected_revision"] = revision.clone();
        bounded(&audit, "保存", save(&f, &issued, &input, 200)).await;
        fs::write(
            f.http.root.join("vault/项目/Alpha/during.md"),
            "# 扫描期间\n",
        )
        .unwrap();
        bounded(&audit, "刷新视图", refresh(&f, &issued)).await;
        audit.release();
    };
    let (page, ()) = tokio::join!(gated, during);
    // 快照在扫描之前定：上界和观察序号都是扫描开始前的，扫描期间的写入和观察不进来。
    assert_eq!(page["through"], earlier["through"]);
    let through = page["through"]["sequence"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let mut seen_paths = Vec::new();
    let mut current = page;
    for _ in 0..20 {
        for row in current["entries"].as_array().unwrap() {
            if row["source"] == "broker" {
                let sequence = row["sequence"].as_str().unwrap().parse::<u64>().unwrap();
                assert!(sequence <= through);
                // 扫描期间落下的已读在本次扫到的范围里，算上：快照里的 Broker 行都已读。
                assert_eq!(row["unread"], false);
            }
            if let Some(path) = row["path"].as_str() {
                seen_paths.push(path.to_owned());
            }
        }
        let Some(next) = current["next_cursor"].as_str() else {
            break;
        };
        // 续页照常能翻（不是 409 快照失效），上界不变。
        current = f
            .get(&format!("{limited}&before={next}"), &issued.cookie, 200)
            .await;
        assert_eq!(current["through"], earlier["through"]);
    }
    assert!(current["next_cursor"].is_null());
    assert!(seen_paths.iter().any(|p| p == "项目/Alpha/before.md"));
    assert!(!seen_paths.iter().any(|p| p == "项目/Alpha/during.md"));
    // 重新读第一页才看得到扫描期间的写入和观察；那次写入不在被标已读的快照里，仍是未读。
    let fresh = f.get(&url(&f), &other.cookie, 200).await;
    let newest = broker_rows(&fresh)
        .into_iter()
        .find(|row| row["sequence"].as_str().unwrap().parse::<u64>().unwrap() == through + 1)
        .expect("扫描期间的写入");
    assert_eq!(newest["unread"], true);
    assert!(
        fresh["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["path"] == "项目/Alpha/during.md")
    );
    f.close().await;
}
