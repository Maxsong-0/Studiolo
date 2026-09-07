//! 累计保留上限和独立安全事件预算，不能被重复拒绝或已终态目标耗尽。

use super::*;
use rusqlite::params;
use sha2::{Digest, Sha256};
use studiolo_audit_ledger::AuditPhase;

#[test]
fn lifetime_capacity_blocks_new_issue_but_existing_session_can_still_revoke() {
    let h = Harness::new(10, 100);
    let caller = h.manager.issue(&identity()).unwrap();
    h.manager.database.transaction(|db|{
        let namespace=crate::session::schema::store_id(db)?;
        // 仅夹具批量建立通过同一规范事件验证的已保留终态，避免1024轮全量验链拖慢契约。
        for index in 1_u64..1024 {
            let issuance=crate::session::credential_store::reserve(db)?;
            let id=SessionId::parse(format!("v2-{issuance:016x}")).unwrap();
            let grant=crate::session::events::Grant{store_id:namespace.clone(),id:id.clone(),epoch:1,
                created:1_000_000,absolute:1_100_000,access_expiry:4102444800000};
            let event=grant.event(AuditPhase::Outcome)?;
            let token=Sha256::digest(format!("token-{index}"));let csrf=Sha256::digest(format!("csrf-{index}"));
            db.execute("INSERT INTO sessions(id,token_digest,csrf_digest,epoch,state,created,last_seen,absolute_expiry,access_expiry,grant_id) VALUES(?1,?2,?3,1,'revoked',1000000,1000000,1100000,4102444800000,?4)",
                params![id.as_str(),token.as_slice(),csrf.as_slice(),event.event_id().as_str()]).unwrap();
            crate::session::credential_store::insert(db,&id,issuance,&token.into(),&csrf.into())?;
            crate::session::outbox::enqueue(db,&event,"issue",None)?;
            for phase in [AuditPhase::Intent,AuditPhase::Outcome] {
                let event=crate::session::events::event(&format!("{namespace}:revoke:{}",id.as_str()),phase,"session.revoked",
                    serde_json::json!({"scope":"one","target":id.as_str(),"revoked":1,"identity_version":1,"effective":"already_revoked"}))?;
                crate::session::outbox::enqueue(db,&event,"revoke",None)?;
            }
        }
        Ok(())
    }).unwrap();
    h.audit
        .fail
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        h.manager.issue(&identity()).unwrap_err(),
        SessionError::Capacity
    );
    let already = SessionId::parse(format!("v2-{:016x}", 2)).unwrap();
    let before = scalar(
        &h.manager,
        "SELECT count(*) FROM audit_outbox WHERE bucket IS NULL",
    );
    assert_eq!(
        h.manager
            .revoke(&identity(), &caller.token, &caller.csrf, &already)
            .unwrap()
            .revoked,
        0
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM audit_outbox WHERE bucket IS NULL"
        ),
        before
    );
    assert_eq!(
        h.manager
            .revoke_all(&identity(), &caller.token, &caller.csrf)
            .unwrap()
            .revoked,
        1
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM audit_outbox WHERE bucket IS NULL"
        ),
        before + 2
    );
}

#[test]
fn cross_domain_random_reuse_is_rejected() {
    use std::sync::atomic::Ordering;
    let h = Harness::new(10, 100);
    let first = h.random.next.load(Ordering::SeqCst);
    h.manager.issue(&identity()).unwrap();
    // 下一次会话秘密被强制取为前一 CSRF；域分离不应让这种源故障逃过碰撞检查。
    h.random.next.store(first + 1, Ordering::SeqCst);
    assert_eq!(
        h.manager.issue(&identity()).unwrap_err(),
        SessionError::RandomCollision
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 1);
}
