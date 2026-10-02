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

#[test]
fn readonly_sqlite_observers_do_not_invalidate_open_owner() {
    for pending in [false, true] {
        let f = Fixture::new();
        let mut store = f.init();
        if pending {
            let original = input(&store, 1340, crate::Presence::Missing, present(9));
            reserve(&mut store, original);
        }
        drop(store);
        let store = f.reopen();
        let before = store.view().unwrap().version;
        let data_before = crate::sqlite::opening::data_version(&store.connection).unwrap();
        for round in 0..3 {
            let external = std::process::Command::new("python3")
                .args([
                    "-I",
                    "-B",
                    "-c",
                    r#"
import sqlite3,sys
p=sys.argv[1]
db=sqlite3.connect('file:'+p+'?mode=ro',uri=True,timeout=3)
db.execute('PRAGMA query_only=ON')
db.execute('BEGIN')
assert db.execute('SELECT revision FROM bridge_meta WHERE id=1').fetchone()
list(db.execute('SELECT sequence,role,ack IS NOT NULL FROM bridge_plans'))
list(db.execute('SELECT payload FROM bridge_actions'))
db.rollback(); db.close()
"#,
                ])
                .arg(f.root.join("bridge.sqlite3"))
                .output()
                .unwrap();
            assert!(external.status.success(), "readonly observer failed");
            let data_after = crate::sqlite::opening::data_version(&store.connection).unwrap();
            assert_eq!(data_before, data_after, "pending={pending} round={round}");
            assert_eq!(store.view().unwrap().version, before);
        }
    }
}

#[test]
fn storage_metadata_checks_preserve_sqlite_writer_lock() {
    let f = Fixture::new();
    let store = f.init();
    store.connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    for _ in 0..3 {
        store.storage.check().unwrap();
        let external = std::process::Command::new("python3")
            .args([
                "-I",
                "-B",
                "-c",
                r#"
import sqlite3,sys
db=sqlite3.connect(sys.argv[1],timeout=0)
try:
    db.execute('BEGIN IMMEDIATE')
except sqlite3.OperationalError as error:
    assert error.sqlite_errorcode == sqlite3.SQLITE_BUSY
else:
    db.rollback()
    raise AssertionError('owner write lock lost')
finally:
    db.close()
"#,
            ])
            .arg(f.root.join("bridge.sqlite3"))
            .output()
            .unwrap();
        assert!(external.status.success(), "外部写事务必须仍被持有者锁阻挡");
    }
    store.connection.execute_batch("ROLLBACK").unwrap();
    assert_eq!(store.view().unwrap().version, 0);
}
