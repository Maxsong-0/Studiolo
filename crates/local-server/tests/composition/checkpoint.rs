//! 从真实启动装配取得同一组 owner；不手工拼一套替代正在服务的实例。
use super::{Fixture, prepare_with, test_support, writer::writer_options};
use std::{fs, num::NonZeroU64, os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc};
use studiolo_audit_ledger::{AuditLedger, SqliteAuditLedger};
use studiolo_backup::worker::{
    job::backup_intent, recovery_point::BackupTargetKind, state::JobKey,
};
use studiolo_core::checkpoint::{
    CheckpointId, CheckpointOwner as Owner, FreezeBudget, FreezeRequest, OwnerAssembly,
    OwnerWatermark,
};
use studiolo_local_server::{
    PolicyPlane,
    checkpoint::{CheckpointInputs, OwnerExportPaths},
};
use studiolo_policy::PolicyStore;
use studiolo_supply::{AgentIdentityView, AgentStore};

fn directory(f: &Fixture, name: &str) -> PathBuf {
    let path = f.root.join(name);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}
fn identities(f: &Fixture, name: &str) -> PathBuf {
    let root = directory(f, name);
    drop(AgentStore::open(&root).unwrap());
    directory(f, &format!("{name}/audit"));
    drop(SqliteAuditLedger::open(&root.join("audit")).unwrap());
    root
}
pub(super) fn policy(
    f: &Fixture,
    options: &mut crate::options::Options,
    agents: bool,
    board: bool,
) {
    let path = directory(f, "policy");
    drop(PolicyStore::open(&path).unwrap());
    options.policy_data = Some(path);
    if agents || board {
        let root = identities(f, "identities");
        if agents {
            options.agent_data = Some(root.clone());
        }
        if board {
            options.board = Some(crate::options::BoardOptions {
                data: directory(f, "control"),
                agent_data: root,
                socket: directory(f, "board-ingress").join("board.sock"),
            });
        }
    }
}
fn export(f: &Fixture, inputs: &CheckpointInputs, agent: bool, policy: bool, board: bool) {
    let key = JobKey {
        sequence: NonZeroU64::new(1).unwrap(),
        checkpoint_id: CheckpointId::parse("startup-original-assembly").unwrap(),
    };
    inputs
        .audit
        .append(backup_intent(&key, BackupTargetKind::LocalTemporaryRepository).unwrap())
        .unwrap();
    let paths = OwnerExportPaths::under(&directory(f, "exports"));
    for path in paths.directories() {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let owners = inputs.bind().unwrap();
    let frozen = owners
        .freeze(
            FreezeRequest {
                checkpoint_id: key.checkpoint_id.clone(),
                budget: FreezeBudget::new(30_000).unwrap(),
            },
            &key,
            BackupTargetKind::LocalTemporaryRepository,
            paths,
        )
        .unwrap();
    let result = frozen
        .release(std::time::Instant::now() + std::time::Duration::from_secs(60))
        .unwrap()
        .export()
        .unwrap();
    for (owner, present) in [
        (Owner::Broker, true),
        (Owner::Auth, true),
        (Owner::Audit, true),
        (Owner::AgentStore, agent),
        (Owner::Policy, policy),
        (Owner::Control, board),
        (Owner::Bridge, false),
    ] {
        assert_eq!(
            result.roster.assembly(owner),
            if present {
                OwnerAssembly::Present
            } else {
                OwnerAssembly::Absent
            }
        );
    }
    let watermark = &result
        .owners
        .iter()
        .find(|o| o.owner == Owner::Audit)
        .unwrap()
        .watermark;
    assert!(
        matches!(watermark, OwnerWatermark::Audit { agent_ledger, .. } if agent_ledger.is_some() == agent)
    );
    assert!(result.audit_coverage.is_exact());
}

#[tokio::test]
async fn readonly_startup_has_no_checkpoint_authority() {
    let f = Fixture::new(true);
    let prepared = prepare_with(f.options(), |_| async {
        Ok(test_support::verifier().await)
    })
    .await
    .unwrap();
    assert!(prepared.checkpoint.is_none());
    f.unbound();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_retains_the_actual_optional_owners_and_exports_their_original_state() {
    for (agent, policy_enabled, board) in [
        (false, false, false),
        (false, true, false),
        (true, true, false),
        (false, true, true),
        (true, true, true),
    ] {
        let f = Fixture::new(true);
        let (mut options, _) = writer_options(&f);
        if policy_enabled {
            policy(&f, &mut options, agent, board);
        }
        let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
            .await
            .unwrap();
        f.unbound();
        let input = prepared.checkpoint.as_ref().unwrap();
        assert!(Arc::ptr_eq(
            &input.workbench,
            prepared.workbench.as_ref().unwrap()
        ));
        if board {
            assert!(Arc::ptr_eq(
                input.board.as_ref().unwrap(),
                prepared.board_plane.as_ref().unwrap()
            ));
        }
        export(&f, input, agent || board, policy_enabled, board);
        input.workbench.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn board_and_policy_identity_mismatch_cannot_silently_omit_one_source() {
    let f = Fixture::new(true);
    let (mut options, _) = writer_options(&f);
    policy(&f, &mut options, true, true);
    let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .unwrap();
    let mut input = prepared.checkpoint.as_ref().unwrap().as_ref().clone();
    let other = identities(&f, "other-identities");
    let other_policy = directory(&f, "other-policy");
    input.policy = Some(Arc::new(
        PolicyPlane::new(
            PolicyStore::open(&other_policy).unwrap(),
            input.audit.clone(),
        )
        .with_identities(AgentIdentityView::open_existing(&other).unwrap()),
    ));
    assert!(input.bind().is_err());
    input.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replaced_original_identity_fails_without_creating_a_new_audit() {
    let f = Fixture::new(true);
    let (mut options, _) = writer_options(&f);
    policy(&f, &mut options, true, false);
    let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .unwrap();
    let input = prepared.checkpoint.as_ref().unwrap();
    let root = f.root.join("identities");
    fs::rename(&root, f.root.join("retained-identities")).unwrap();
    let replacement = identities(&f, "identities");
    fs::rename(replacement.join("audit"), f.root.join("unused-audit")).unwrap();
    assert!(input.bind().is_err());
    assert!(!root.join("audit").exists());
    input.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listener_router_keeps_the_exact_startup_checkpoint_sources() {
    use axum::{
        Extension, Router,
        body::{Body, to_bytes},
        http::Request,
        routing::get,
    };
    use tower::ServiceExt;
    async fn inspect(Extension(input): Extension<Arc<CheckpointInputs>>) -> &'static str {
        assert!(input.bind().is_ok());
        "original-sources"
    }
    let f = Fixture::new(true);
    let (options, _) = writer_options(&f);
    let mut prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .unwrap();
    let workbench = prepared.workbench.as_ref().unwrap().clone();
    prepared.app = Router::new().route("/test-only-sources", get(inspect));
    let (listener, app) = prepared.bind().await.unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/test-only-sources")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        &to_bytes(response.into_body(), 100).await.unwrap()[..],
        b"original-sources"
    );
    drop(listener);
    workbench.shutdown().await.unwrap();
}
