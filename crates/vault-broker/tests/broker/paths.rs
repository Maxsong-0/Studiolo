//! 保护域、大小写/规范化碰撞、链接与特殊文件一律失败关闭。

use std::fs;

use studiolo_vault_broker::{
    BrokerError, FsVaultBroker, MemoryVaultBroker, Mutation, VaultReader, VaultWriter,
};
use tempfile::TempDir;

use super::{artifact_request, broker, path};

#[test]
fn protected_paths_and_collisions_are_rejected() {
    let (_vault, _data, mut broker) = broker();
    assert!(matches!(
        broker.mutate(Mutation::create(
            path(".workbench/vault.json"),
            b"forged".to_vec(),
            "forge"
        )),
        Err(BrokerError::ProtectedPath(_))
    ));

    broker
        .mutate(Mutation::create(
            path("Notes/A.md"),
            b"one".to_vec(),
            "case-1",
        ))
        .expect("first spelling");
    assert!(matches!(
        broker.mutate(Mutation::create(
            path("notes/a.md"),
            b"two".to_vec(),
            "case-2"
        )),
        Err(BrokerError::PathCollision { .. })
    ));
}

#[test]
fn decomposed_entry_names_on_disk_collide_with_their_nfc_spelling() {
    let (vault, _data, mut broker) = broker();
    // 外部写者（HFS+ 时代或别的工具）留下 NFD 拼写：か + U+3099，与 NFC 的 が 是同一逻辑名。
    fs::create_dir_all(vault.path().join("notes")).unwrap();
    fs::write(vault.path().join("notes/か\u{3099}.md"), b"decomposed").unwrap();
    assert!(matches!(
        broker.mutate(Mutation::create(
            path("notes/が.md"),
            b"composed".to_vec(),
            "nfc-create"
        )),
        Err(BrokerError::PathCollision { .. })
    ));
    assert!(matches!(
        broker.read(&path("notes/が.md")),
        Err(BrokerError::PathCollision { .. })
    ));
}

#[cfg(unix)]
#[test]
fn symlink_hardlink_and_special_files_fail_closed() {
    use std::os::unix::{fs::symlink, net::UnixListener};

    let (vault, _data, mut broker) = broker();
    let outside =
        TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).expect("outside tempdir");
    fs::write(outside.path().join("secret.md"), b"outside").expect("outside file");
    symlink(outside.path(), vault.path().join("escape")).expect("create symlink");
    assert!(matches!(
        broker.read(&path("escape/secret.md")),
        Err(BrokerError::UnsafeNode { .. })
    ));

    let hard_receipt = broker
        .mutate(Mutation::create(path("hard.md"), b"one".to_vec(), "hard-1"))
        .expect("create hardlink source");
    fs::hard_link(
        vault.path().join("hard.md"),
        vault.path().join("hard-copy.md"),
    )
    .expect("create hardlink");
    assert!(matches!(
        broker.mutate(Mutation::replace(
            path("hard.md"),
            b"two".to_vec(),
            hard_receipt.revision,
            "hard-2"
        )),
        Err(BrokerError::UnsafeNode { .. })
    ));

    let socket = vault.path().join("socket");
    let bound = UnixListener::bind(&socket);
    // 分支自证：部分应用沙箱禁止创建 Unix socket；打印判据输入与实际 bind 结果。
    eprintln!(
        "socket 夹具·分支自证：路径 {} 字节，bind = {:?}",
        socket.as_os_str().len(),
        bound.as_ref().map(|_| ())
    );
    match bound {
        Ok(_socket) => assert!(matches!(
            broker.read(&path("socket")),
            Err(BrokerError::UnsafeNode { .. })
        )),
        // Linux 测试机没有沙箱限制，必须走上一支；只有其他平台的沙箱可以跳过。
        Err(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied
                && !cfg!(target_os = "linux") =>
        {
            eprintln!("跳过：当前沙箱禁止创建 Unix socket（{error}），socket 拒绝分支本次未验证");
        }
        Err(error) => panic!("bind unix socket: {error}"),
    }
}

#[cfg(unix)]
#[test]
fn vault_identity_cannot_be_redirected_through_a_symlink() {
    use std::os::unix::fs::symlink;

    let vault = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let data = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let outside = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    fs::create_dir(vault.path().join(".workbench")).unwrap();
    fs::write(outside.path().join("vault.json"), b"{}").unwrap();
    symlink(
        outside.path().join("vault.json"),
        vault.path().join(".workbench/vault.json"),
    )
    .unwrap();

    assert!(matches!(
        FsVaultBroker::open(vault.path(), data.path()),
        Err(BrokerError::CorruptIdentity)
    ));
}

#[test]
fn memory_double_rejects_aliasing_parent_names_and_protected_artifact_names() {
    let mut broker = MemoryVaultBroker::new();
    broker
        .mutate(Mutation::create(path("Notes/a.md"), b"a".to_vec(), "first"))
        .unwrap();
    assert!(matches!(
        broker.mutate(Mutation::create(
            path("notes/b.md"),
            b"b".to_vec(),
            "second"
        )),
        Err(BrokerError::PathCollision { .. })
    ));
    let mut commit = artifact_request();
    commit.artifacts[0].name = path(".workbench/private.md");
    assert!(matches!(
        broker.commit_attempt_artifacts(commit),
        Err(BrokerError::InvalidArtifact(_))
    ));
}
