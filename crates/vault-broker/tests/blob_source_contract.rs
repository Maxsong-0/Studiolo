//! 来源必须有明确字节上限，根/路径变化不能被一个仍然打开的旧 FD 掩盖。
use std::{fs, io::Write};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{BlobSource, BrokerError, StreamLimits};
use tempfile::TempDir;

fn temporary() -> TempDir {
    tempfile::Builder::new()
        .prefix("studiolo-t046-blob-")
        .tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap())
        .unwrap()
}
fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}
fn limit(value: u64) -> StreamLimits {
    StreamLimits::new(value).unwrap()
}

#[test]
fn precise_binary_identity_and_limit() {
    let root = temporary();
    fs::write(
        root.path().join("payload.bin"),
        (0..=255u8).collect::<Vec<_>>(),
    )
    .unwrap();
    let source = BlobSource::open(root.path(), path("payload.bin")).unwrap();
    let value = source.identity(limit(256)).unwrap();
    assert_eq!(value.bytes, 256);
    assert_eq!(
        value.sha256,
        "40aff2e9d2d8922e47afd4648e6967497158785fbd1da870e7110266bf944880"
    );
    assert!(matches!(
        source.identity(limit(255)),
        Err(BrokerError::WriterLimitExceeded)
    ));
}

#[test]
fn explicit_limits_do_not_invent_a_service_cap() {
    assert!(StreamLimits::new(0).is_err());
    assert!(StreamLimits::new(256 * 1024 * 1024).is_ok());
    assert!(StreamLimits::new(256 * 1024 * 1024 + 1).is_err());
    let root = temporary();
    fs::write(root.path().join("empty.bin"), []).unwrap();
    let source = BlobSource::open(root.path(), path("empty.bin")).unwrap();
    assert_eq!(source.identity(limit(1)).unwrap().bytes, 0);
    assert!(
        source
            .identity(StreamLimits { effective_limit: 0 })
            .is_err()
    );
}

#[test]
fn hidden_and_linked_sources_are_refused() {
    use std::os::unix::fs::symlink;
    let root = temporary();
    fs::write(root.path().join("original.bin"), b"original").unwrap();
    fs::write(root.path().join(".hidden"), b"hidden").unwrap();
    assert!(BlobSource::open(root.path(), path(".hidden")).is_err());
    symlink("original.bin", root.path().join("link.bin")).unwrap();
    assert!(BlobSource::open(root.path(), path("link.bin")).is_err());
    fs::hard_link(
        root.path().join("original.bin"),
        root.path().join("alias.bin"),
    )
    .unwrap();
    assert!(BlobSource::open(root.path(), path("original.bin")).is_err());
}

#[test]
fn root_path_rebinding_does_not_read_an_old_detached_root() {
    let parent = temporary();
    let root = parent.path().join("selected");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("payload.bin"), b"same").unwrap();
    let source = BlobSource::open(&root, path("payload.bin")).unwrap();
    fs::rename(&root, parent.path().join("old")).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(root.join("payload.bin"), b"same").unwrap();
    assert!(source.identity(limit(4)).is_err());
}

#[test]
fn replacement_with_special_node_is_refused_without_blocking() {
    let root = temporary();
    fs::write(root.path().join("payload.bin"), b"before").unwrap();
    let source = BlobSource::open(root.path(), path("payload.bin")).unwrap();
    fs::remove_file(root.path().join("payload.bin")).unwrap();
    // rustix 的 mkfifoat 不提供 macOS 实现；两平台均用系统工具造临时夹具。
    assert!(
        std::process::Command::new("/usr/bin/mkfifo")
            .arg(root.path().join("payload.bin"))
            .status()
            .unwrap()
            .success()
    );
    assert!(source.identity(limit(64)).is_err());
}

#[test]
fn non_sparse_200_mib_plus_17_bytes_are_fully_hashed() {
    let root = temporary();
    let mut file = fs::File::create(root.path().join("large.bin")).unwrap();
    let chunk: Vec<u8> = (0..65536).map(|i| ((i * 17 + 3) % 251) as u8).collect();
    let bytes = 200 * 1024 * 1024 + 17;
    for _ in 0..bytes / chunk.len() {
        file.write_all(&chunk).unwrap();
    }
    file.write_all(&chunk[..bytes % chunk.len()]).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let source = BlobSource::open(root.path(), path("large.bin")).unwrap();
    let value = source.identity(limit(bytes as u64)).unwrap();
    assert_eq!(value.bytes, bytes as u64);
    assert_eq!(
        value.sha256,
        "1606f5a7d61b07583745b90864f05b332d60140b876c93025d07e1b1baaa13db"
    );
}
