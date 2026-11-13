//! 人侧/接入方复用授权规则，整份列表的吊销信息不得在事务中撕裂。
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};
use studiolo_policy::{Denial, PolicyAgentStore, PolicyStore, RevocationLevel};
const NOW: &str = "2026-11-13T00:00:00Z";
#[test]
fn owner_and_agent_authorization_agree_and_batch_repeats_one_snapshot() {
    let dir = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let owner = PolicyStore::open(dir.path()).unwrap();
    let agent = PolicyAgentStore::open_existing(dir.path()).unwrap();
    let identities = BTreeMap::from([
        ("parent".into(), "claude".into()),
        ("child".into(), "claude".into()),
    ]);
    for level in [
        None,
        Some(RevocationLevel::Instance),
        Some(RevocationLevel::Type),
        Some(RevocationLevel::Global),
    ] {
        if let Some(level) = level {
            owner
                .revoke(
                    level,
                    if level == RevocationLevel::Instance {
                        "parent"
                    } else if level == RevocationLevel::Type {
                        "claude"
                    } else {
                        "*"
                    },
                    "owner",
                    "停用",
                    NOW,
                )
                .unwrap();
        }
        let snapshot = owner
            .agent_snapshot(&["claude".into()], &identities)
            .unwrap();
        for id in identities.keys() {
            let actual = owner.authorization("claude", Some(id)).unwrap();
            assert_eq!(actual, agent.authorization("claude", Some(id)).unwrap());
            assert_eq!(snapshot.instances[id].denial, actual.denial);
            assert_eq!(snapshot.effective_hash, actual.effective_hash);
        }
    }
}
#[test]
fn concurrent_revocations_never_split_list_flags_from_denials() {
    let dir = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let reader = PolicyStore::open(dir.path()).unwrap();
    let writer = PolicyStore::open_existing(dir.path()).unwrap();
    let identities = (0..500)
        .map(|n| (format!("instance-{n}"), "claude".into()))
        .collect();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            for _ in 0..30 {
                writer
                    .revoke(RevocationLevel::Global, "*", "owner", "暂停", NOW)
                    .unwrap();
                writer.release_global("owner", NOW).unwrap();
            }
        });
        for _ in 0..10 {
            let snapshot = reader
                .agent_snapshot(&["claude".into()], &identities)
                .unwrap();
            let expected = if snapshot.global_revoked {
                Denial::GlobalRevoked
            } else {
                Denial::NoCapabilitySet
            };
            assert!(
                snapshot
                    .instances
                    .values()
                    .all(|state| state.denial == Some(expected))
            );
        }
    });
}
