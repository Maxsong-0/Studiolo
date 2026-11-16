use super::*;
use studiolo_policy::{DesiredInput, RevocationLevel};
use studiolo_supply::AgentStore;
const AGENTS: &str = "/api/v1/agents";
const NOW: &str = "2026-11-13T00:00:00Z";
fn row<'a>(value: &'a Value, id: &str) -> &'a Value {
    value["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap()
}
async fn listing(f: &Fixture, login: &Login) -> Value {
    f.send(f.request("GET", AGENTS, Some(login), ""), 200)
        .await
        .json()
}
#[tokio::test]
async fn agents_are_closed_read_only_snapshots_with_six_denials_and_parent_links() {
    let mut f = Fixture::with_identities(true, true).await;
    f.send(f.request("GET", AGENTS, None, ""), 401).await;
    let login = f.issue().await;
    f.send(f.request("POST", AGENTS, Some(&login), "{}"), 405)
        .await;
    f.send(
        f.request("GET", &format!("{AGENTS}?x=1"), Some(&login), ""),
        400,
    )
    .await;
    let writer = AgentStore::open_existing(&f.root.join("agents")).unwrap();
    writer.issue_type("claude", NOW).unwrap();
    writer.upgrade_child_instances().unwrap();
    let parent = writer
        .register_instance("claude", "parent-secret", NOW)
        .unwrap();
    let child = writer
        .register_child_instance("claude", &parent.id, "child-secret", NOW)
        .unwrap();
    let data = listing(&f, &login).await;
    assert_eq!(data.as_object().unwrap().len(), 7);
    assert_eq!(data["types"][0].as_object().unwrap().len(), 5);
    assert_eq!(row(&data, &child.id).as_object().unwrap().len(), 7);
    assert_eq!(
        row(&data, &child.id)["denial"]["reason"],
        "no_capability_set"
    );
    assert_eq!(row(&data, &child.id)["parent"], parent.id);
    for secret in [
        "token_sha256",
        "session_digest",
        "parent-secret",
        "child-secret",
    ] {
        assert!(!data.to_string().contains(secret));
    }
    let policy = f.policy();
    let body = json!({"ai_type":"claude", "read":{"project_whitelist":["项目/Studiolo"],"deny_by_default":true},"write":{"board":false,"content_patch_only":true,"content_direct":false},"policy":{"propose_desired":false,"activate":false},"hard_bound_skills":[]}).to_string();
    let activation = policy
        .bootstrap(
            DesiredInput {
                kind: PolicyKind::CapabilitySet,
                subject: "claude".into(),
                body: Some(body),
            },
            NOW,
        )
        .unwrap();
    let data = listing(&f, &login).await;
    assert_eq!(data["effective_hash"], activation.effective_hash);
    assert!(row(&data, &child.id)["denial"].is_null());
    policy
        .revoke(
            RevocationLevel::Instance,
            &parent.id,
            "owner",
            "父实例停用",
            NOW,
        )
        .unwrap();
    let data = listing(&f, &login).await;
    assert_eq!(
        row(&data, &parent.id)["denial"]["reason"],
        "instance_revoked"
    );
    assert_eq!(
        row(&data, &child.id)["denial"],
        json!({"reason":"ancestor_revoked", "by":parent.id})
    );
    policy
        .revoke(RevocationLevel::Type, "claude", "owner", "类型停用", NOW)
        .unwrap();
    assert_eq!(
        row(&listing(&f, &login).await, &child.id)["denial"]["reason"],
        "type_revoked"
    );
    policy
        .revoke(RevocationLevel::Global, "*", "owner", "暂停", NOW)
        .unwrap();
    assert_eq!(
        row(&listing(&f, &login).await, &child.id)["denial"]["reason"],
        "global_revoked"
    );
    writer.revoke_type("claude", NOW).unwrap();
    assert_eq!(
        row(&listing(&f, &login).await, &child.id)["denial"]["reason"],
        "identity_type_revoked"
    );
    fs::rename(f.root.join("agents"), f.root.join("agents-old")).unwrap();
    fs::create_dir(f.root.join("agents")).unwrap();
    let unavailable = f
        .send(f.request("GET", AGENTS, Some(&login), ""), 503)
        .await;
    assert!(!String::from_utf8_lossy(&unavailable.body).contains(f.root.to_str().unwrap()));
    f.stop().await;
}
#[tokio::test]
async fn absent_identity_view_is_distinct_from_empty_and_instances_are_bounded() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let absent = f
        .send(f.request("GET", AGENTS, Some(&login), ""), 404)
        .await
        .json();
    assert_eq!(absent["code"], "agents_plane_absent");
    f.stop().await;
    let mut f = Fixture::with_identities(true, true).await;
    let login = f.issue().await;
    assert_eq!(listing(&f, &login).await["instances"], json!([]));
    let writer = AgentStore::open_existing(&f.root.join("agents")).unwrap();
    writer.issue_type("claude", NOW).unwrap();
    for n in 0..501 {
        writer
            .register_instance("claude", &format!("session-{n}"), NOW)
            .unwrap();
    }
    let data = listing(&f, &login).await;
    assert_eq!(data["instances"].as_array().unwrap().len(), 500);
    assert_eq!(data["instances_truncated"], true);
    assert_eq!(data["children_enabled"], false);
    assert!(
        data["instances"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["parent"].is_null())
    );
    f.stop().await;
}
