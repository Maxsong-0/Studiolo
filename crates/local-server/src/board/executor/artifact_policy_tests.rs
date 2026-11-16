//! 固定配置批准及实际来源留存，不启动真实执行体或访问外网。
use super::*;
fn project() -> ProjectConfig {
    serde_json::from_value(serde_json::json!({
        "project":"projects/fixture", "repository":"/fixture/repository", "repository_tree":"a".repeat(40),
        "repository_sha256":"b".repeat(64), "repository_egress":"deny",
        "approved_instruction_sha256":["c".repeat(64)], "instruction_egress":"deny",
        "allow_full_artifact_backup":true, "allow_artifact_desktop_sync":true,
        "retain_until_unix_seconds":30, "sources":[],
    })).unwrap()
}
fn sources() -> Vec<RelaySource> {
    vec![RelaySource {
        source: "fixture-input".into(),
        revision: "d".repeat(64),
        sha256: "e".repeat(64),
        retain_until_unix_seconds: 20,
        model_egress: studiolo_gateway::ModelEgress::Deny,
    }]
}
#[test]
fn artifact_approval_uses_shortest_actual_source_retention_and_does_not_classify_output_from_input()
{
    let project = project();
    let fact = original(&project, &"f".repeat(64), "original-snapshot", &sources()).unwrap();
    assert_eq!(fact.retain_until_unix_seconds(), 20);
    assert_eq!(fact.labels().classification(), Classification::Restricted);
    assert_eq!(fact.labels().backup_scope(), BackupScope::Full);
    assert_eq!(fact.labels().desktop_sync(), DesktopSync::Deny);
    assert_eq!(fact.labels().model_egress(), &ModelEgress::Deny);
    assert_eq!(
        fact,
        original(&project, &"f".repeat(64), "original-snapshot", &sources()).unwrap()
    );
    assert_ne!(
        fact.approval_sha256(),
        original(&project, &"f".repeat(64), "other-snapshot", &sources())
            .unwrap()
            .approval_sha256()
    );
    let mut changed = sources();
    changed[0].sha256 = "0".repeat(64);
    assert_ne!(
        fact.approval_sha256(),
        original(&project, &"f".repeat(64), "original-snapshot", &changed)
            .unwrap()
            .approval_sha256()
    );
    let mut shorter = project.clone();
    shorter.retain_until_unix_seconds = 10;
    assert_eq!(
        original(&shorter, &"f".repeat(64), "original-snapshot", &sources())
            .unwrap()
            .retain_until_unix_seconds(),
        10
    );
}
#[test]
fn artifact_approval_requires_original_explicit_config_and_valid_source_policy() {
    let mut project = project();
    project.allow_full_artifact_backup = false;
    assert!(original(&project, &"f".repeat(64), "snapshot", &sources()).is_err());
    project.allow_full_artifact_backup = true;
    project.allow_artifact_desktop_sync = false;
    assert!(original(&project, &"f".repeat(64), "snapshot", &sources()).is_err());
    project.allow_artifact_desktop_sync = true;
    assert!(original(&project, "latest", "snapshot", &sources()).is_err());
    assert!(original(&project, &"f".repeat(64), "snapshot", &[]).is_err());
    project.retain_until_unix_seconds = 0;
    assert!(original(&project, &"f".repeat(64), "snapshot", &sources()).is_err());
}
