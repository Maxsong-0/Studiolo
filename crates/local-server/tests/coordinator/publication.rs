use std::{fs, sync::atomic::Ordering};

use studiolo_core::VaultPath;
use studiolo_vault_broker::{BrokerError, Mutation, VaultWriter};

use super::fixture::{Fixture, entered, idle};
use crate::LocalError;

#[tokio::test]
async fn a_failed_candidate_does_not_consume_a_newer_repair_request() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    let before = workbench.core.view.snapshot().unwrap().generation;
    let captures = workbench.core.probe.captures.load(Ordering::SeqCst);
    fs::write(fixture.root.join("vault/note.md"), [0xff]).unwrap();
    let (observed, release) = workbench.core.probe.arm("building");
    workbench.core.request("changed").unwrap();
    entered(observed).await;
    fs::write(fixture.root.join("vault/note.md"), "# 已修复的虚构内容\n").unwrap();
    let repair = workbench.core.request("changed").unwrap();
    release.send(()).unwrap();
    workbench.core.wait(repair).await.unwrap();
    idle(&workbench).await;
    let after = workbench.core.view.snapshot().unwrap();
    assert_eq!(after.generation.number, before.number + 1);
    assert_eq!(
        workbench.core.probe.captures.load(Ordering::SeqCst),
        captures + 2
    );
    assert!(!after.freshness.unwrap().stale);
    workbench.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_failed_scan_retains_the_entire_previous_view() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    let before = workbench.core.view.snapshot().unwrap().generation;
    fs::write(fixture.root.join("vault/note.md"), [0xff]).unwrap();
    assert!(workbench.refresh().await.is_err());
    idle(&workbench).await;
    let after = workbench.core.view.snapshot().unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, &after.generation));
    assert!(after.freshness.unwrap().stale);
    workbench.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_single_read_marks_an_observed_unresolved_journal_stale_immediately() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    let mut journal = fixture.journal();
    journal["legacy_unresolved"] = true.into();
    fixture.replace_journal(&journal);
    assert!(matches!(
        workbench.read_file(&VaultPath::parse("note.md").unwrap(), 4096),
        Err(LocalError::Read(BrokerError::MutationUnresolved))
    ));
    let freshness = workbench.core.view.snapshot().unwrap().freshness.unwrap();
    assert!(freshness.stale);
    assert_eq!(freshness.unresolved, Some(true));
    workbench.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_single_read_marks_an_observed_journal_rollback_stale_immediately() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    let previous = fixture.journal();
    let path = VaultPath::parse("note.md").unwrap();
    {
        let mut owner = workbench.core.owner.lock().unwrap();
        let broker = owner.as_mut().unwrap();
        let old = broker.read_file(&path, 4096).unwrap();
        broker
            .mutate(Mutation::replace(
                path.clone(),
                b"# later\n".to_vec(),
                old.revision,
                "advance-before-rollback",
            ))
            .unwrap();
    }
    workbench.refresh().await.unwrap();
    idle(&workbench).await;
    fixture.replace_journal(&previous);
    assert!(matches!(
        workbench.read_file(&path, 4096),
        Err(LocalError::Read(BrokerError::CorruptWriterState))
    ));
    assert!(
        workbench
            .core
            .view
            .snapshot()
            .unwrap()
            .freshness
            .unwrap()
            .stale
    );
    workbench.shutdown().await.unwrap();
}

#[tokio::test]
async fn ordinary_read_rejections_preserve_freshness_but_a_known_deletion_invalidates_it() {
    let fixture = Fixture::new();
    let workbench = fixture.start(false).await;
    assert!(
        workbench
            .read_file(&VaultPath::parse("missing.md").unwrap(), 4096)
            .is_err()
    );
    assert!(
        workbench
            .read_file(&VaultPath::parse("note.md").unwrap(), 0)
            .is_err()
    );
    assert!(
        !workbench
            .core
            .view
            .snapshot()
            .unwrap()
            .freshness
            .unwrap()
            .stale
    );
    fs::remove_file(fixture.root.join("vault/note.md")).unwrap();
    let (observed, release) = workbench.core.probe.arm("capture");
    assert!(
        workbench
            .read_file(&VaultPath::parse("note.md").unwrap(), 4096)
            .is_err()
    );
    entered(observed).await;
    assert!(
        workbench
            .core
            .view
            .snapshot()
            .unwrap()
            .freshness
            .unwrap()
            .stale
    );
    workbench.stop();
    release.send(()).unwrap();
    workbench.shutdown().await.unwrap();
}
