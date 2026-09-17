//! 真实临时目录与 root FD 副本；不依赖 fork 调度或重开重试。

use super::{acquire, walk};
use crate::{BrokerError, FsVaultBroker};
use cap_std::fs::Dir;
use std::path::PathBuf;
use tempfile::TempDir;

struct Fixture {
    _temporary: TempDir,
    vault: PathBuf,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temporary = TempDir::new().unwrap();
        // macOS 临时路径可能含 /var 别名；仅夹具转为实际路径。
        let base = temporary.path().canonicalize().unwrap();
        let fixture = Self {
            _temporary: temporary,
            vault: base.join("vault"),
            data: base.join("data"),
        };
        drop(FsVaultBroker::open(&fixture.vault, &fixture.data).unwrap());
        fixture
    }

    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.vault, &self.data).unwrap()
    }

    fn assert_busy(&self) {
        assert!(matches!(
            FsVaultBroker::open_existing(&self.vault, &self.data),
            Err(BrokerError::WriterBusy)
        ));
    }
}

#[test]
fn owner_drop_releases_lease_while_root_clone_is_alive() {
    let fixture = Fixture::new();
    let owner = fixture.open();
    // root 和租约来自同一次独立 open；这是实际 Broker 字段的副本。
    let inherited = owner.root.try_clone().unwrap();
    fixture.assert_busy();
    // 第一个竞争者失败后的 Drop 不能解除原 owner 的锁。
    fixture.assert_busy();
    drop(owner);

    // 副本仍存活，且没有 sleep/retry；新 owner 必须立即取得租约。
    let next = fixture.open();
    fixture.assert_busy();
    drop(inherited);
    // 旧 open-file-description 副本关闭不能解除新 owner 的锁。
    fixture.assert_busy();
    drop(next);
    drop(fixture.open());
}

#[test]
fn construction_error_releases_guard_with_a_retained_root_clone() {
    let fixture = Fixture::new();
    let missing = fixture.data.join("missing-data-root");
    let mut inherited: Option<Dir> = None;
    let result = (|| -> Result<(), BrokerError> {
        // 模拟 opening 的真实次序：根 -> 成功租约 -> data 打开失败。
        let root = walk(&fixture.vault, false)?;
        let _lease = acquire(&root)?;
        inherited = Some(
            root.try_clone()
                .map_err(|error| BrokerError::io("retain test root", error))?,
        );
        let _data = walk(&missing, false)?;
        Ok(())
    })();
    assert!(result.is_err());
    assert!(inherited.is_some());
    assert!(!missing.exists());

    // 构造错误已经离开作用域，原 root 副本仍在，不能把锁遗留给它。
    let next = fixture.open();
    fixture.assert_busy();
    drop(inherited);
    fixture.assert_busy();
    drop(next);
    drop(fixture.open());
}
