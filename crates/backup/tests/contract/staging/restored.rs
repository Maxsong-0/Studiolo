//! 取回副本：把一份真实封存整棵复制到别处，只读核验，再逐字节核对着放到新目录。
use super::{fixture::*, *};
use std::path::{Path, PathBuf};
use studiolo_backup::checkpoint::staging::{Placement, RestoredCheckpoint};

fn sealed(f: &Fixture) -> SealedCheckpoint {
    let area = f.area("restore");
    let note = f.object(&area, "canonical/note", b"note bytes", BackupScope::Full);
    private_dir(&area.export_root().join("canonical/nested"));
    let deep = f.object(
        &area,
        "canonical/nested/deep",
        b"deeper bytes",
        BackupScope::Full,
    );
    area.seal(&SourceCatalog::new([note, deep]).unwrap(), f.inputs())
        .unwrap()
}
/// 模拟「从远端取回」：整棵复制到别处，目录 0500、文件 0400。
fn fetch(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            fetch(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o400)).unwrap();
        }
    }
    fs::set_permissions(to, fs::Permissions::from_mode(0o500)).unwrap();
}
fn fetched(f: &Fixture, sealed: &SealedCheckpoint, name: &str) -> PathBuf {
    let copy = f.temp.path().join(name);
    fetch(&sealed.ticket().transfer_source.root, &copy);
    copy
}
fn rewrite(path: &Path, bytes: &[u8]) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o400)).unwrap();
}
fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().mode() & 0o7777
}

#[test]
fn fetched_copy_verifies_at_any_path_but_never_becomes_an_upload_source() {
    let f = Fixture::new();
    let sealed = sealed(&f);
    // 放到另一个父目录下、连目录名都和原封存一样：只读核验通过，按上传来源重开仍然被拒。
    let other = f.temp.path().join("other");
    private_dir(&other);
    let copy = SealedCheckpoint::root_path(&other, &checkpoint("restore"));
    fetch(&sealed.ticket().transfer_source.root, &copy);
    let restored = RestoredCheckpoint::open(&copy, sealed.reference(), deadline()).unwrap();
    assert_eq!(restored.manifest(), sealed.manifest());
    assert_eq!(restored.reference(), sealed.reference());
    assert!(matches!(
        SealedCheckpoint::open(
            &other,
            sealed.reference(),
            InstantMillis::from_millis(1001),
            deadline()
        ),
        Err(StagingError::Manifest)
    ));
}

#[test]
fn fetched_copy_is_rejected_when_reference_bytes_shape_or_mode_differ() {
    let f = Fixture::new();
    let sealed = sealed(&f);

    // 引用不是这份封存的：副本自己报的摘要不算数。
    let copy = fetched(&f, &sealed, "wrong-reference");
    let mut foreign = sealed.reference().clone();
    foreign.seal_sha256 = "ab".repeat(32);
    assert!(matches!(
        RestoredCheckpoint::open(&copy, &foreign, deadline()),
        Err(StagingError::Manifest)
    ));

    // 正文被改了一个字节（长度不变）。
    let copy = fetched(&f, &sealed, "tampered");
    rewrite(&copy.join("data/canonical/note"), b"note bytez");
    assert!(matches!(
        RestoredCheckpoint::open(&copy, sealed.reference(), deadline()),
        Err(StagingError::Changed)
    ));

    // 多出一个清单里没有的文件。
    let copy = fetched(&f, &sealed, "extra");
    let nested = copy.join("data/canonical/nested");
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(nested.join("planted"), b"x").unwrap();
    fs::set_permissions(nested.join("planted"), fs::Permissions::from_mode(0o400)).unwrap();
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o500)).unwrap();
    assert!(matches!(
        RestoredCheckpoint::open(&copy, sealed.reference(), deadline()),
        Err(StagingError::Changed)
    ));

    // 少了一个文件。
    let copy = fetched(&f, &sealed, "missing");
    let nested = copy.join("data/canonical/nested");
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_file(nested.join("deep")).unwrap();
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o500)).unwrap();
    assert!(matches!(
        RestoredCheckpoint::open(&copy, sealed.reference(), deadline()),
        Err(StagingError::Changed)
    ));

    // 副本还能被写：不是只读的取证副本。
    let copy = fetched(&f, &sealed, "writable");
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        RestoredCheckpoint::open(&copy, sealed.reference(), deadline()),
        Err(StagingError::UnsafeRoot)
    ));
}

#[test]
fn extraction_writes_only_verified_bytes_and_never_overwrites() {
    let f = Fixture::new();
    let sealed = sealed(&f);
    let copy = fetched(&f, &sealed, "fetched");
    let restored = RestoredCheckpoint::open(&copy, sealed.reference(), deadline()).unwrap();
    let target = f.temp.path().join("new-root");
    private_dir(&target);

    assert!(!RestoredCheckpoint::placed(&target, "vault").unwrap());
    assert_eq!(
        restored.extract("canonical", &target, "vault", deadline()),
        Ok(Placement::Extracted)
    );
    assert!(RestoredCheckpoint::placed(&target, "vault").unwrap());
    assert_eq!(fs::read(target.join("vault/note")).unwrap(), b"note bytes");
    assert_eq!(
        fs::read(target.join("vault/nested/deep")).unwrap(),
        b"deeper bytes"
    );
    // 新目录要能继续写：目录 0700、文件 0600；中间名字不留下。
    assert_eq!(mode(&target.join("vault")), 0o700);
    assert_eq!(mode(&target.join("vault/nested")), 0o700);
    assert_eq!(mode(&target.join("vault/note")), 0o600);
    assert!(!target.join("vault.restoring").exists());

    // 已经有的名字不覆盖，也不重新核对：原样告诉调用方。
    fs::write(target.join("vault/note"), b"written later").unwrap();
    assert_eq!(
        restored.extract("canonical", &target, "vault", deadline()),
        Ok(Placement::AlreadyPresent)
    );
    assert_eq!(
        fs::read(target.join("vault/note")).unwrap(),
        b"written later"
    );

    // 清单里没有的组件、带分隔符的名字都拒绝。
    assert_eq!(
        restored.extract("nope", &target, "x", deadline()),
        Err(StagingError::Catalog)
    );
    assert!(
        restored
            .extract("canonical/nested", &target, "x", deadline())
            .is_err()
    );
    assert!(
        restored
            .extract("canonical", &target, "a/b", deadline())
            .is_err()
    );
    assert!(!target.join("x").exists());

    // 上一次中途失败留下的中间目录不自动清，下次直接拒绝。
    private_dir(&target.join("again.restoring"));
    assert_eq!(
        restored.extract("canonical", &target, "again", deadline()),
        Err(StagingError::CleanupRequired)
    );
    assert!(!target.join("again").exists());

    // 核验之后副本又被动过：写到一半发现摘要不对，最终名字不出现。
    rewrite(&copy.join("data/canonical/note"), b"note bytez");
    assert_eq!(
        restored.extract("canonical", &target, "late", deadline()),
        Err(StagingError::Changed)
    );
    assert!(!target.join("late").exists());
}

/// 上一次放置中途留下的 `<名字>.restoring`，不管是目录、普通文件还是悬空的符号链接都算在场，编排据此在写任何
/// 东西之前拒绝；不存在才算没被打断（复审 A2-F2）。判断不跟随链接：悬空链接要是被当成不存在，前面的组件写完了
/// 才在 extract 里撞上。
#[test]
fn an_interrupted_placement_is_seen_in_every_shape_without_following_links() {
    let f = Fixture::new();
    let parent = f.temp.path().join("placement");
    private_dir(&parent);
    let partial = parent.join("auth.restoring");
    let interrupted = || RestoredCheckpoint::interrupted(&parent, "auth").unwrap();
    assert!(!interrupted(), "不存在");

    fs::create_dir(&partial).unwrap();
    assert!(interrupted(), "目录");
    fs::remove_dir(&partial).unwrap();

    private_file(&partial, b"half written");
    assert!(interrupted(), "普通文件");
    fs::remove_file(&partial).unwrap();

    std::os::unix::fs::symlink(parent.join("missing"), &partial).unwrap();
    assert!(fs::metadata(&partial).is_err(), "夹具：链接确实是悬空的");
    assert!(interrupted(), "悬空符号链接");
    assert!(
        !RestoredCheckpoint::interrupted(&parent, "audit").unwrap(),
        "别的名字不受影响"
    );
    assert!(!RestoredCheckpoint::placed(&parent, "auth").unwrap());
}
