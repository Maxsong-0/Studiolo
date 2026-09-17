//! 真实 rename 后变成不安全节点仍是未决安装，不能返回普通路径拒绝。

use super::*;

#[test]
fn post_install_unsafe_node_is_unresolved_and_retry_never_infers_success() {
    for replace in [false, true] {
        let fixture = Fixture::new();
        let mut broker = fixture.initialize();
        let mutation = if replace {
            let first = broker
                .mutate(Mutation::create(
                    path("note.md"),
                    b"old".to_vec(),
                    "initial",
                ))
                .unwrap();
            Mutation::replace(path("note.md"), b"intended".to_vec(), first.revision, "key")
        } else {
            request()
        };
        let watermark = broker.write_status().unwrap().committed_through;
        interrupt(&broker, 17);
        assert!(matches!(
            broker.mutate(mutation.clone()),
            Err(BrokerError::MutationUnresolved)
        ));
        assert_eq!(broker.writer.fault.load(Ordering::Acquire), 0);
        assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"intended");
        assert!(matches!(
            broker.read(&path("note.md")),
            Err(BrokerError::MutationUnresolved)
        ));
        let operation = load(&broker).unwrap().operation.unwrap();
        assert_eq!(operation.phase, Phase::Installing);
        assert!(operation.receipt.is_none());
        assert!(!fixture.stage(&broker).exists());
        let journal = fs::read(fixture.data.join("writer-state.json")).unwrap();
        assert!(broker.mutation_replay(&mutation).unwrap().is_none());
        assert!(matches!(
            broker.mutate(mutation.clone()),
            Err(BrokerError::MutationUnresolved)
        ));
        assert_eq!(
            fs::read(fixture.data.join("writer-state.json")).unwrap(),
            journal
        );
        // 修复节点安全性也不是原安装完成的证据，重开必须保留 Installing。
        fs::remove_file(fixture.root.join(".studiolo-t046-after-rename-hardlink")).unwrap();
        drop(broker);
        let mut broker = fixture.open();
        assert_eq!(broker.writer.fault.load(Ordering::Acquire), 0);
        let status = broker.write_status().unwrap();
        assert_eq!(status.phase, crate::WriterPhase::Installing);
        assert_eq!(status.committed_through, watermark);
        assert!(status.unresolved && status.read_blocked);
        assert!(matches!(
            broker.mutate(mutation),
            Err(BrokerError::MutationUnresolved)
        ));
        assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"intended");
        assert_eq!(
            fs::read(fixture.data.join("writer-state.json")).unwrap(),
            journal
        );
    }
}
