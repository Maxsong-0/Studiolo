use crate::fixture::Fixture;

#[tokio::test]
async fn observed_unresolved_writer_marks_last_complete_generation_stale_before_read_returns() {
    let f = Fixture::new(7, false).await;
    let issued = f.http.issue().await;
    let before = f.get("/api/workbench", &issued.cookie, 200).await;
    let mut state = f.journal();
    state["legacy_unresolved"] = true.into();
    f.set_journal(&state);
    let error = f.get("/api/v1/file?path=a.md", &issued.cookie, 503).await;
    assert!(error.get("content").is_none());
    assert!(!error.to_string().contains(f.http.root.to_str().unwrap()));
    let stale = f.get("/api/workbench", &issued.cookie, 200).await;
    assert_eq!(stale["generation"], before["generation"]);
    assert_eq!(stale["freshness"]["stale"], true);
    assert_eq!(stale["freshness"]["indexed_through"], "7");
    assert!(stale["freshness"]["committed_through"].is_null());
    f.get("/api/workbench?refresh=true", &issued.cookie, 503)
        .await;
    let retained = f.get("/api/workbench", &issued.cookie, 200).await;
    assert_eq!(retained["generation"], before["generation"]);
    assert_eq!(retained["freshness"]["stale"], true);
    f.close().await;
}

#[tokio::test]
async fn observed_watermark_rollback_cannot_be_hidden_by_restoring_the_newer_log() {
    let f = Fixture::new(12, false).await;
    let issued = f.http.issue().await;
    let previous = f.journal();
    let mut rollback = previous.clone();
    rollback["committed_through"] = 11.into();
    f.set_journal(&rollback);
    f.get("/api/v1/file?path=a.md", &issued.cookie, 503).await;
    f.set_journal(&previous);
    f.get("/api/v1/file?path=a.md", &issued.cookie, 503).await;
    let body = f.get("/api/workbench", &issued.cookie, 200).await;
    assert_eq!(body["freshness"]["stale"], true);
    assert!(body["freshness"]["committed_through"].is_null());
    f.get("/api/workbench?refresh=true", &issued.cookie, 503)
        .await;
    f.close().await;
}

#[tokio::test]
async fn oversized_private_journal_is_a_global_fault_and_keeps_the_last_complete_view() {
    let f = Fixture::new(4, false).await;
    let issued = f.http.issue().await;
    let before = f.get("/api/workbench", &issued.cookie, 200).await;
    std::fs::write(f.data.join("writer-state.json"), vec![b' '; 64 * 1024 + 1]).unwrap();
    f.get("/api/v1/file?path=a.md", &issued.cookie, 503).await;
    let stale = f.get("/api/workbench", &issued.cookie, 200).await;
    assert_eq!(stale["generation"], before["generation"]);
    assert_eq!(stale["freshness"]["stale"], true);
    assert!(stale["freshness"]["committed_through"].is_null());
    f.close().await;
}

#[tokio::test]
async fn replaced_private_state_directory_is_a_global_fault_without_following_the_link() {
    let f = Fixture::new(4, false).await;
    let issued = f.http.issue().await;
    let original = f.data.join("idempotency");
    let retained = f.data.join("retained-idempotency");
    std::fs::rename(&original, &retained).unwrap();
    std::os::unix::fs::symlink(&retained, &original).unwrap();
    f.get("/api/v1/file?path=a.md", &issued.cookie, 503).await;
    let stale = f.get("/api/workbench", &issued.cookie, 200).await;
    assert_eq!(stale["freshness"]["stale"], true);
    assert!(stale["freshness"]["committed_through"].is_null());
    assert_eq!(std::fs::read_dir(retained).unwrap().count(), 0);
    f.close().await;
}
