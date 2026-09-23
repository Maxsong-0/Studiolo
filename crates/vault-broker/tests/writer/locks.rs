use super::support::{Fixture, inventory, path};
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use studiolo_vault_broker::{
    BrokerError, CanonicalReadVault, FsVaultBroker, Mutation, ReadOnlyVault, VaultWriter,
};

#[test]
fn independent_handles_cannot_bypass_the_writer_lock_with_another_data_root() {
    let fixture = Fixture::initialized();
    let first = fixture.open();
    let other = fixture.data.with_file_name("other-data");
    fs::create_dir(&other).unwrap();
    let before = inventory(&other);
    assert!(matches!(
        FsVaultBroker::open_existing(&fixture.vault, &fixture.data),
        Err(BrokerError::WriterBusy)
    ));
    assert!(matches!(
        FsVaultBroker::open(&fixture.vault, &other),
        Err(BrokerError::WriterBusy)
    ));
    assert_eq!(inventory(&other), before);
    drop(first);
    drop(fixture.open());
    // 释放锁不等于授权重绑到另一份日志。
    assert!(matches!(
        FsVaultBroker::open(&fixture.vault, &other),
        Err(BrokerError::WriterBindingMismatch)
    ));
}

#[test]
fn readers_share_the_root_without_acquiring_writer_authority() {
    let fixture = Fixture::initialized();
    let mut writer = fixture.open();
    writer
        .mutate(Mutation::create(path("note.md"), b"one".to_vec(), "one"))
        .unwrap();
    let canonical = CanonicalReadVault::open_existing(&fixture.vault).unwrap();
    let readonly = ReadOnlyVault::open(&fixture.vault).unwrap();
    assert_eq!(
        canonical.read_file(&path("note.md"), 16).unwrap().content,
        b"one"
    );
    drop(writer);
    let mut reopened = fixture.open();
    reopened
        .mutate(Mutation::create(path("other.md"), b"two".to_vec(), "two"))
        .unwrap();
    assert_eq!(
        canonical.read_file(&path("other.md"), 16).unwrap().content,
        b"two"
    );
    drop(readonly);
}

/// 子进程入口的精确测试名，父测试以 `--ignored --exact` 只运行这一项。
const WRITER_CHILD: &str = "locks::writer_child";

/// 子进程参数一律来自父测试自有夹具；缺失即说明不是父测试调起，必须失败而不是计为通过。
fn child_argument(name: &str) -> std::ffi::OsString {
    std::env::var_os(name)
        .unwrap_or_else(|| panic!("{WRITER_CHILD} 只能由父测试以子进程方式调起：缺少 {name}"))
}

// 子进程只运行当前测试二进制中的这一入口，所有参数都是父测试自有夹具路径。
#[test]
#[ignore = "仅由父测试以子进程方式调起"]
fn writer_child() {
    let vault = child_argument("STUDIOLO_WRITER_TEST_VAULT");
    let data = child_argument("STUDIOLO_WRITER_TEST_DATA");
    let ready = child_argument("STUDIOLO_WRITER_TEST_READY");
    let release = child_argument("STUDIOLO_WRITER_TEST_RELEASE");
    let writer = FsVaultBroker::open_existing(Path::new(&vault), Path::new(&data)).unwrap();
    fs::write(ready, b"locked").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !Path::new(&release).exists() {
        assert!(Instant::now() < deadline, "parent did not release child");
        thread::sleep(Duration::from_millis(10));
    }
    drop(writer);
}

struct OwnedChild(Child);

impl OwnedChild {
    fn wait_ready(&mut self, ready: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(
                self.0.try_wait().unwrap().is_none(),
                "child exited before locking"
            );
            assert!(
                Instant::now() < deadline,
                "child did not acquire writer lock"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_success(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success(), "writer child failed: {status}");
                return;
            }
            assert!(Instant::now() < deadline, "writer child did not exit");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        // 包含父测试断言失败路径；仅终止并回收本夹具创建的子进程。
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[test]
fn real_child_holds_the_root_lock_until_drop_or_process_exit() {
    for termination in ["drop", "kill"] {
        let fixture = Fixture::initialized();
        let ready = fixture.directory.path().join("child-ready");
        let release = fixture.directory.path().join("child-release");
        let other = fixture.data.with_file_name("other-data");
        fs::create_dir(&other).unwrap();
        let mut child = OwnedChild(
            Command::new(std::env::current_exe().unwrap())
                .args(["--ignored", "--exact", WRITER_CHILD, "--nocapture"])
                .env("STUDIOLO_WRITER_TEST_VAULT", &fixture.vault)
                .env("STUDIOLO_WRITER_TEST_DATA", &fixture.data)
                .env("STUDIOLO_WRITER_TEST_READY", &ready)
                .env("STUDIOLO_WRITER_TEST_RELEASE", &release)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        child.wait_ready(&ready);
        assert!(matches!(
            FsVaultBroker::open_existing(&fixture.vault, &fixture.data),
            Err(BrokerError::WriterBusy)
        ));
        assert!(matches!(
            FsVaultBroker::open(&fixture.vault, &other),
            Err(BrokerError::WriterBusy)
        ));
        if termination == "drop" {
            fs::write(&release, b"release").unwrap();
            child.wait_success();
        } else {
            // 只验证 OS 在进程退出时释放锁；这里没有在途写入，不是断电恢复测试。
            child.0.kill().unwrap();
            assert!(!child.0.wait().unwrap().success());
        }
        drop(fixture.open());
    }
}
