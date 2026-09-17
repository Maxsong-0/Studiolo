use super::support::*;
use crate::sqlite::*;
use sha2::{Digest as _, Sha256};

fn digests(f: &Fixture) -> ([u8; 32], [u8; 32]) {
    let main = std::fs::read(f.root.join("bridge.sqlite3")).unwrap();
    let wal = std::fs::read(f.root.join("bridge.sqlite3-wal")).unwrap();
    assert!(wal.len() > 32, "夹具必须持有尚未 checkpoint 的真实 WAL");
    (Sha256::digest(main).into(), Sha256::digest(wal).into())
}

#[test]
fn rejected_open_does_not_checkpoint_valid_wal_or_rewrite_persistent_configuration() {
    for variant in 0..3 {
        let f = Fixture::new();
        drop(f.init());
        // 保留第二连接，不让 SQLite 最后连接关闭的清理掩盖被测打开行为。
        let db = rusqlite::Connection::open(f.root.join("bridge.sqlite3")).unwrap();
        db.execute_batch("PRAGMA wal_autocheckpoint=0;").unwrap();
        let mut expected = binding();
        let failure = match variant {
            0 => {
                db.execute("UPDATE bridge_meta SET revision=revision+1", [])
                    .unwrap();
                expected.scope_epoch += 1;
                BridgeError::BindingMismatch
            }
            1 => {
                db.execute("UPDATE bridge_meta SET encoded_total=encoded_total+1", [])
                    .unwrap();
                BridgeError::Budget
            }
            _ => {
                db.execute_batch("PRAGMA application_id=19;").unwrap();
                BridgeError::CorruptStore
            }
        };
        let before = digests(&f);
        let result = BridgeState::open_existing(&f.root, expected);
        assert!(matches!(result,Err(ref error) if error==&failure));
        assert_eq!(digests(&f), before, "错误打开不得主动改变主库或 WAL 字节");
        drop(db);
    }
}

#[test]
fn validated_open_can_checkpoint_and_preserve_the_loaded_revision() {
    let f = Fixture::new();
    drop(f.init());
    let db = rusqlite::Connection::open(f.root.join("bridge.sqlite3")).unwrap();
    db.execute_batch("PRAGMA wal_autocheckpoint=0;").unwrap();
    db.execute("UPDATE bridge_meta SET revision=revision+1", [])
        .unwrap();
    digests(&f);
    let store = f.reopen();
    assert_eq!(store.view().unwrap().version, 1);
    assert_eq!(
        std::fs::metadata(f.root.join("bridge.sqlite3-wal"))
            .unwrap()
            .len(),
        0
    );
}
