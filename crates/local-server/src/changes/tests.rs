use super::*;
#[test]
fn seen_accepts_only_server_snapshot_reference_and_a_single_project_scope() {
    let mut value = serde_json::json!({"vault_uuid":VaultUuid::new(),"snapshot":VaultUuid::new(),"request_id":VaultUuid::new(),"project":"项目/Alpha"});
    assert!(parse_seen(&serde_json::to_vec(&value).unwrap()).is_ok());
    value["sequence"] = serde_json::json!(999999);
    assert!(parse_seen(&serde_json::to_vec(&value).unwrap()).is_err());
    value.as_object_mut().unwrap().remove("sequence");
    for project in ["项目/Alpha/nested", "Alpha", "项目/", "项目", "CLAUDE.md"] {
        value["project"] = serde_json::json!(project);
        assert!(
            parse_seen(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{project}"
        );
    }
}
