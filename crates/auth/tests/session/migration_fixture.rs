//! 使用真实 Auth 连接和原四表格式建立固定虚构 v1 活跃与待激活会话。

use super::*;
use rusqlite::{Connection, TransactionBehavior, params, types::Value};
use studiolo_audit_ledger::{AuditLedger, AuditPhase};

pub(super) struct LegacyFixture {
    pub h: Harness,
    pub id: SessionId,
    pub token: SessionToken,
    pub csrf: CsrfToken,
    pub pending_token: SessionToken,
}

impl LegacyFixture {
    pub fn new() -> Self {
        let h = Harness::new(10, 100);
        let token = SessionToken::from_hex("11".repeat(32)).unwrap();
        let csrf = CsrfToken::from_hex("22".repeat(32)).unwrap();
        let pending_token = SessionToken::from_hex("44".repeat(32)).unwrap();
        let pending_csrf = CsrfToken::from_hex("55".repeat(32)).unwrap();
        let id = SessionId::parse("33".repeat(16)).unwrap();
        {
            let mut connection = h.manager.database.connection.lock().unwrap();
            let db = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            // 新库未签发任何凭证；移除附加表后逐字结构就是受支持的旧版本。
            db.execute_batch("DROP TABLE auth_retention_plan;DROP TABLE auth_retention_checkpoint;DROP TABLE session_seeds;DROP TABLE auth_legacy;DROP TABLE credential_state;PRAGMA user_version=1;").unwrap();
            grant(&db, &h, &id, &token, &csrf, true);
            grant(
                &db,
                &h,
                &SessionId::parse("66".repeat(16)).unwrap(),
                &pending_token,
                &pending_csrf,
                false,
            );
            schema::validate(&db, &config(10, 100).encoded().unwrap()).unwrap();
            db.commit().unwrap();
        }
        Self {
            h,
            id,
            token,
            csrf,
            pending_token,
        }
    }

    pub fn migrate(&self) -> SessionManager {
        SessionManager::migrate_v1(
            &self.h.root,
            config(10, 100),
            self.h.audit.clone(),
            self.h.clock.clone(),
            self.h.random.clone(),
        )
        .unwrap()
    }

    pub fn snapshot(&self) -> Vec<Vec<Vec<Value>>> {
        snapshot(&self.h.manager.database.connection.lock().unwrap())
    }
}

fn grant(
    db: &Connection,
    h: &Harness,
    id: &SessionId,
    token: &SessionToken,
    csrf: &CsrfToken,
    active: bool,
) {
    let grant = events::Grant {
        store_id: schema::store_id(db).unwrap(),
        id: id.clone(),
        epoch: 1,
        created: 1_000_000,
        absolute: 1_100_000,
        access_expiry: state::access_expiry(&identity()).unwrap(),
    };
    h.audit
        .append(grant.event(AuditPhase::Intent).unwrap())
        .unwrap();
    let event = grant.event(AuditPhase::Outcome).unwrap();
    let receipt =
        active.then(|| outbox::receipt(&h.audit.append(event.clone()).unwrap(), &event).unwrap());
    outbox::enqueue(db, &event, "issue", None).unwrap();
    db.execute(
        "UPDATE audit_outbox SET receipt_json=?1 WHERE event_id=?2",
        params![&receipt, event.event_id().as_str()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO sessions VALUES(?1,?2,?3,1,?4,1000000,1000000,1100000,?5,?6,?7,?8)",
        params![
            id.as_str(),
            secrets::token_digest(token).as_slice(),
            secrets::csrf_digest(csrf).as_slice(),
            if active { "active" } else { "pending" },
            grant.access_expiry,
            event.event_id().as_str(),
            receipt,
            active.then_some(1_000_000_i64)
        ],
    )
    .unwrap();
}

pub(super) fn snapshot(db: &Connection) -> Vec<Vec<Vec<Value>>> {
    ["auth_meta", "sessions", "audit_outbox", "auth_denials"]
        .iter()
        .map(|table| {
            let mut statement = db
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let columns = statement.column_count();
            statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|column| row.get::<_, Value>(column))
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
        .collect()
}
