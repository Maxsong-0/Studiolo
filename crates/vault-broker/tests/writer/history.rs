use super::support::{Fixture, path};
use std::{fs, os::unix::fs::symlink};
use studiolo_vault_broker::{BrokerError, Mutation, VaultReader, VaultWriter};

#[test]
fn existing_history_must_be_exact_unlinked_bytes_before_replacement() {
    for damage in ["different-bytes", "symlink", "hardlink", "directory"] {
        let fixture = Fixture::initialized();
        let mut broker = fixture.open();
        let first = broker
            .mutate(Mutation::create(
                path("note.md"),
                b"original".to_vec(),
                "initial",
            ))
            .unwrap();
        let history = fixture
            .data
            .join("history")
            .join(first.revision.to_string());
        let outside = fixture.directory.path().join("outside-original");
        // 当前版本也已作为L0对象保存；先明确替换它，再制造各类损坏。
        assert_eq!(fs::read(&history).unwrap(), b"original");
        fs::remove_file(&history).unwrap();
        match damage {
            "different-bytes" => fs::write(&history, b"forged").unwrap(),
            "directory" => fs::create_dir(&history).unwrap(),
            _ => {
                fs::write(&outside, b"original").unwrap();
                if damage == "symlink" {
                    symlink(&outside, &history).unwrap();
                } else {
                    fs::hard_link(&outside, &history).unwrap();
                }
            }
        }
        let result = broker.mutate(Mutation::replace(
            path("note.md"),
            b"replacement".to_vec(),
            first.revision,
            "replace",
        ));
        // L0 历史对象按 blob 打开：内容不符是状态损坏，链接、硬链接与目录按节点拒绝。
        let rejected = match damage {
            "different-bytes" => matches!(&result, Err(BrokerError::CorruptWriterState)),
            "symlink" => matches!(
                &result,
                Err(BrokerError::UnsafeNode {
                    reason: "no-link blob open failed",
                    ..
                })
            ),
            "hardlink" => matches!(
                &result,
                Err(BrokerError::UnsafeNode {
                    reason: "hard-linked or unlinked file",
                    ..
                })
            ),
            _ => matches!(
                &result,
                Err(BrokerError::UnsafeNode {
                    reason: "not a regular file",
                    ..
                })
            ),
        };
        assert!(rejected, "accepted {damage} history: {:?}", result.err());
        assert_eq!(broker.read(&path("note.md")).unwrap().content, b"original");
        if outside.exists() {
            assert_eq!(fs::read(outside).unwrap(), b"original");
        }
    }
}
