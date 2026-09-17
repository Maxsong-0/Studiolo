//! 小字节真实临时文件契约；ACK 使用既有 Broker 测试 helper，不冒充真实 AuditLedger。
mod blocked;
mod outcomes;

use super::*;
use std::{collections::BTreeMap, path::Path};

// 测试夹具只含本用例的小文件；快照同时覆盖正文、目录、stage、描述符与水位日志。
fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, at: &Path, result: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(at).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if entry.file_type().unwrap().is_dir() {
                result.insert(relative, None);
                visit(root, &path, result);
            } else {
                result.insert(relative, Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn fresh_write(
    broker: &mut FsVaultBroker,
    request: Mutation,
    recorded: bool,
) -> Result<MutationReceipt, BrokerError> {
    if recorded {
        broker.mutate_recorded(request, metadata())
    } else {
        broker.mutate(request)
    }
}

fn assert_blocked(fixture: &Fixture, broker: &mut FsVaultBroker) {
    assert!(broker.content_audit_status().unwrap().restore_unconfirmed);
    assert_eq!(broker.write_status().unwrap().phase, WriterPhase::Ready);
    let before = tree(fixture._temp.path());
    let status = broker.write_status().unwrap();
    let current = broker.read(&path()).unwrap();
    for recorded in [false, true] {
        for create in [false, true] {
            let key = format!("blocked-{recorded}-{create}");
            let request = if create {
                Mutation::create(
                    VaultPath::parse("must-not-appear/note.md").unwrap(),
                    b"C".to_vec(),
                    key,
                )
            } else {
                Mutation::replace(path(), b"C".to_vec(), current.revision, key)
            };
            assert!(matches!(
                fresh_write(broker, request, recorded),
                Err(BrokerError::MutationUnresolved)
            ));
            assert_eq!(tree(fixture._temp.path()), before);
            assert_eq!(broker.write_status().unwrap(), status);
        }
    }
}

fn assert_old_replays(fixture: &Fixture, broker: &mut FsVaultBroker, first: Revision) {
    let before = tree(fixture._temp.path());
    let old = request(b"A", "first");
    assert_eq!(
        broker.mutation_replay(&old).unwrap().unwrap().revision,
        first
    );
    assert_eq!(
        broker
            .mutation_replay_recorded(&old, &metadata())
            .unwrap()
            .unwrap()
            .revision,
        first
    );
    // 两个兼容安装入口命中原终态后必须直接返回，不受另一恢复的新写门禁影响。
    assert_eq!(broker.mutate(old.clone()).unwrap().revision, first);
    assert_eq!(
        broker.mutate_recorded(old, metadata()).unwrap().revision,
        first
    );
    assert_eq!(tree(fixture._temp.path()), before);
}

fn assert_resumed(fixture: &Fixture, broker: &mut FsVaultBroker) {
    assert!(!broker.content_audit_status().unwrap().restore_unconfirmed);
    for recorded in [false, true] {
        let path = VaultPath::parse(format!("resumed-{recorded}/note.md")).unwrap();
        let request = Mutation::create(path.clone(), b"C".to_vec(), format!("resume-{recorded}"));
        let before = broker.write_status().unwrap().committed_through;
        let receipt = fresh_write(broker, request, recorded).unwrap();
        assert_eq!(receipt.path, path);
        assert_eq!(fs::read(fixture.root.join(path.as_str())).unwrap(), b"C");
        // 取消过的Prepared可占用序号，提交水位必须前进但不是提交次数。
        assert!(broker.write_status().unwrap().committed_through > before);
    }
}

fn install_restore(
    broker: &mut FsVaultBroker,
    source: Revision,
    expected: Revision,
    streaming: bool,
    interrupted: bool,
) -> Result<MutationReceipt, BrokerError> {
    if streaming {
        let ticket = broker
            .prepare_stream_restore(
                path(),
                expected,
                source,
                "restore".into(),
                actor(),
                StreamLimits::new(16).unwrap(),
            )
            .unwrap();
        broker
            .confirm_stream_restore_intent(&ticket, 1, &"a".repeat(64))
            .unwrap();
        if interrupted {
            interrupt(broker, 3);
        }
        broker.commit_stream_mutation(ticket)
    } else {
        let (request, metadata) = restore_request(source, expected, "restore");
        let ticket = broker.prepare_recorded_restore(request, metadata).unwrap();
        broker
            .confirm_recorded_restore_intent(&ticket, 1, &"a".repeat(64))
            .unwrap();
        if interrupted {
            interrupt(broker, 3);
        }
        broker.commit_recorded_restore(ticket)
    }
}

fn replay_restore(
    broker: &FsVaultBroker,
    source: Revision,
    expected: Revision,
    streaming: bool,
) -> Result<Option<MutationReceipt>, BrokerError> {
    if streaming {
        broker.stream_mutation_replay(
            &StreamMutation {
                path: path(),
                precondition: Precondition::Match(expected),
                idempotency_key: "restore".into(),
                content: BlobIdentity {
                    bytes: 1,
                    sha256: crate::sha256(b"A"),
                },
            },
            &MutationMetadata::new(
                actor(),
                MutationKind::Restore {
                    source_revision: source,
                },
            )
            .unwrap(),
        )
    } else {
        let (request, metadata) = restore_request(source, expected, "restore");
        broker.mutation_replay_recorded(&request, &metadata)
    }
}
