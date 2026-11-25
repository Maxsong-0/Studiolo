//! 真实私有存储契约：只在本次 /tmp 建固定三个名字，不碰任何共享目录或服务。

use super::support::*;
use crate::owned_run::*;
use crate::store::paths;
use crate::store::publish::PublishFault;
use crate::store::{FilePublisher, StatePublisher};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

struct Root {
    _temp: tempfile::TempDir,
    path: PathBuf,
}

impl Root {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").expect("/tmp 可规范化"))
            .expect("可以建临时目录");
        let path = temp.path().join("transport");
        fs::create_dir(&path).expect("可以建私有根");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("可以设私有权限");
        Self { _temp: temp, path }
    }
}

/// 同一次测试里两份确定的完整记录；安装标识只生成一次。
struct Records {
    first: Vec<u8>,
    second: Vec<u8>,
}

fn records() -> Records {
    let approval = FakeApproval::new();
    let runtime = PinnedRuntime::from_approval(&approval).expect("固定运行时可取");
    let state = initial_state(&approval, &runtime, OWNER).expect("首条记录可构造");
    let first = state::encode(&state).expect("首条记录可编码");
    let second = state::encode(&TransportState {
        generation: 2,
        ..state
    })
    .expect("第二条记录可编码");
    Records { first, second }
}

fn names(root: &Path) -> Vec<String> {
    let mut entries = fs::read_dir(root)
        .expect("可以列目录")
        .map(|entry| {
            entry
                .expect("可以读目录项")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

#[test]
fn an_uninitialised_private_root_is_never_completed_by_opening_it() {
    let root = Root::new();
    assert_eq!(
        FilePublisher::open_existing(&root.path).err(),
        Some(TransportError::MissingStore)
    );
    assert!(names(&root.path).is_empty());
}

#[test]
fn initialisation_is_an_explicit_action_that_refuses_to_run_twice() {
    let records = records();
    let root = Root::new();
    let publisher = FilePublisher::initialize(&root.path, &records.first).expect("可以显式初始化");
    assert_eq!(publisher.read().expect("可以读回"), records.first);
    assert_eq!(
        names(&root.path),
        vec![paths::STATE.to_owned(), paths::TRANSPORT_LOCK.to_owned()]
    );
    drop(publisher);

    assert_eq!(
        FilePublisher::initialize(&root.path, &records.first).err(),
        Some(TransportError::AlreadyExists)
    );
}

#[test]
fn a_published_record_survives_a_reopen_and_reads_back_byte_for_byte() {
    let records = records();
    let root = Root::new();
    let mut publisher =
        FilePublisher::initialize(&root.path, &records.first).expect("可以显式初始化");
    publisher.replace(&records.second).expect("可以完整替换");
    assert_eq!(publisher.read().expect("可以读回"), records.second);
    drop(publisher);

    let reopened = FilePublisher::open_existing(&root.path).expect("可以重开");
    assert_eq!(reopened.read().expect("可以读回"), records.second);
    assert_eq!(
        names(&root.path),
        vec![paths::STATE.to_owned(), paths::TRANSPORT_LOCK.to_owned()]
    );
}

#[test]
fn a_failed_publish_removes_its_own_temporary_and_the_next_publish_succeeds() {
    for fault in [PublishFault::ShortWrite, PublishFault::SyncFailure] {
        let records = records();
        let root = Root::new();
        let mut publisher =
            FilePublisher::initialize(&root.path, &records.first).expect("可以显式初始化");
        assert_eq!(
            publisher.replace_faulted(&records.second, fault).err(),
            Some(TransportError::Unavailable)
        );
        // 权威记录保持旧版本；报错的那一次自己建的临时文件已经收走。
        assert_eq!(publisher.read().expect("可以读回"), records.first);
        assert_eq!(
            names(&root.path),
            vec![paths::STATE.to_owned(), paths::TRANSPORT_LOCK.to_owned()],
            "{fault:?}"
        );
        publisher
            .replace(&records.second)
            .expect("下一次发布照常能成");
        drop(publisher);
        let reopened = FilePublisher::open_existing(&root.path).expect("可以重开");
        assert_eq!(reopened.read().expect("可以读回"), records.second);
    }
}

#[test]
fn a_failed_publish_never_removes_a_temporary_it_did_not_create() {
    let records = records();
    let root = Root::new();
    let mut publisher =
        FilePublisher::initialize(&root.path, &records.first).expect("可以显式初始化");
    let residue = root.path.join(paths::STATE_PUBLISH_TEMP);
    fs::write(&residue, b"someone else").expect("可以放一份不是自己建的");
    fs::set_permissions(&residue, fs::Permissions::from_mode(0o600)).expect("可以设权限");
    assert_eq!(
        publisher.replace(&records.second).err(),
        Some(TransportError::TemporaryResidue)
    );
    assert_eq!(fs::read(&residue).expect("残留原样"), b"someone else");
    // 报错的那一刻临时文件已经被换成别人的：不是自己那个 inode 就不碰。
    fs::remove_file(&residue).expect("可以移走");
    assert_eq!(
        publisher
            .replace_faulted(&records.second, PublishFault::ReplacedMidWrite)
            .err(),
        Some(TransportError::Unavailable)
    );
    assert_eq!(fs::read(&residue).expect("别人的原样"), b"");
    assert_eq!(publisher.read().expect("可以读回"), records.first);
}

#[test]
fn discarding_rechecks_that_the_root_and_the_residue_are_still_the_opened_ones() {
    for shape in ["replaced", "gone", "root"] {
        let records = records();
        let root = Root::new();
        drop(FilePublisher::initialize(&root.path, &records.first).expect("可以显式初始化"));
        let path = root.path.join(paths::STATE_PUBLISH_TEMP);
        let leave = |bytes: &[u8]| {
            fs::write(&path, bytes).expect("可以摆残留");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("可以设权限");
        };
        leave(b"opened");
        let (publisher, residue) =
            FilePublisher::open_with_residue(&root.path).expect("形态合规的残留可以打开");
        let residue = residue.expect("有残留");
        let moved = root.path.with_file_name("moved");
        // 打开着的描述符不放，旧 inode 号不会被新文件复用。
        let expected = match shape {
            "replaced" => {
                fs::rename(&path, &moved).expect("可以移走");
                leave(b"replaced");
                TransportError::UnsafeStorage
            }
            "gone" => {
                fs::remove_file(&path).expect("可以移走");
                TransportError::MissingStore
            }
            _ => {
                fs::rename(&root.path, &moved).expect("可以移走原根");
                fs::create_dir(&root.path).expect("可以建同名新根");
                fs::set_permissions(&root.path, fs::Permissions::from_mode(0o700))
                    .expect("可以设私有权限");
                TransportError::UnsafeStorage
            }
        };
        assert_eq!(publisher.discard(&residue).err(), Some(expected), "{shape}");
        match shape {
            "replaced" => assert_eq!(fs::read(&path).expect("别人的原样"), b"replaced"),
            "root" => assert_eq!(
                fs::read(moved.join(paths::STATE_PUBLISH_TEMP)).expect("移走的原样"),
                b"opened"
            ),
            _ => {}
        }
    }
}

#[test]
fn a_killed_publish_leaves_a_residue_that_is_never_published_as_authorised_state() {
    for fault in [
        PublishFault::KilledMidWrite,
        PublishFault::KilledBeforeRename,
    ] {
        let records = records();
        let root = Root::new();
        let mut publisher =
            FilePublisher::initialize(&root.path, &records.first).expect("可以显式初始化");
        assert_eq!(
            publisher.replace_faulted(&records.second, fault).err(),
            Some(TransportError::Unavailable)
        );
        // 权威记录保持旧版本；临时文件只是残留，不是授权状态。
        assert_eq!(publisher.read().expect("可以读回"), records.first);
        assert!(names(&root.path).contains(&paths::STATE_PUBLISH_TEMP.to_owned()));
        assert_eq!(
            publisher.replace(&records.second).err(),
            Some(TransportError::TemporaryResidue)
        );
        drop(publisher);
        // 原始打开照旧不认领残留；核过权威记录之后才丢的入口见 `residue.rs`。
        assert_eq!(
            FilePublisher::open_existing(&root.path).err(),
            Some(TransportError::TemporaryResidue)
        );
        assert_eq!(
            fs::read(root.path.join(paths::STATE)).expect("权威记录还在"),
            records.first
        );
    }
}

#[test]
fn a_second_owner_cannot_take_the_exclusive_lease() {
    let records = records();
    let root = Root::new();
    let first = FilePublisher::initialize(&root.path, &records.first).expect("可以显式初始化");
    assert_eq!(
        FilePublisher::open_existing(&root.path).err(),
        Some(TransportError::Busy)
    );
    drop(first);
    FilePublisher::open_existing(&root.path).expect("释放后可以重开");
}

#[test]
fn a_root_that_is_not_a_private_directory_is_refused() {
    let records = records();
    let root = Root::new();
    FilePublisher::initialize(&root.path, &records.first).expect("可以显式初始化");
    fs::set_permissions(&root.path, fs::Permissions::from_mode(0o755)).expect("可以改权限");
    assert_eq!(
        FilePublisher::open_existing(&root.path).err(),
        Some(TransportError::UnsafeStorage)
    );

    for bad in ["relative/path", "/srv/../etc", "/srv/./x"] {
        assert_eq!(
            FilePublisher::open_existing(Path::new(bad)).err(),
            Some(TransportError::UnsafeStorage),
            "{bad}"
        );
    }
}

#[test]
fn a_record_over_the_state_budget_never_touches_the_previous_version() {
    let records = records();
    let root = Root::new();
    let mut publisher =
        FilePublisher::initialize(&root.path, &records.first).expect("可以显式初始化");
    let oversized = vec![b'x'; STATE_MAX_BYTES + 1];
    assert_eq!(
        publisher.replace(&oversized).err(),
        Some(TransportError::Budget)
    );
    assert_eq!(publisher.replace(&[]).err(), Some(TransportError::Budget));
    assert_eq!(publisher.read().expect("可以读回"), records.first);
    assert!(!names(&root.path).contains(&paths::STATE_PUBLISH_TEMP.to_owned()));
}

#[test]
fn a_replaced_root_object_is_detected_before_anything_else_is_written() {
    let records = records();
    let root = Root::new();
    let mut publisher =
        FilePublisher::initialize(&root.path, &records.first).expect("可以显式初始化");
    let moved = root.path.with_file_name("moved");
    fs::rename(&root.path, &moved).expect("可以移走原根");
    fs::create_dir(&root.path).expect("可以建同名新根");
    fs::set_permissions(&root.path, fs::Permissions::from_mode(0o700)).expect("可以设私有权限");

    assert_eq!(publisher.check().err(), Some(TransportError::UnsafeStorage));
    assert_eq!(publisher.read().err(), Some(TransportError::UnsafeStorage));
    assert_eq!(
        publisher.replace(&records.second).err(),
        Some(TransportError::UnsafeStorage)
    );
    assert!(names(&root.path).is_empty());
}

/// 原地删掉再建同名根：Linux 会把同一个 inode 号立刻发还，光比 `(dev, ino)` 看不出替换。
/// 持有的根 FD 必须认出「批准的那个目录已经没了」，读写都不得落到新目录上。
#[test]
fn a_root_deleted_and_recreated_in_place_is_detected_even_when_the_inode_is_recycled() {
    let records = records();
    let root = Root::new();
    let mut publisher =
        FilePublisher::initialize(&root.path, &records.first).expect("可以显式初始化");
    for name in names(&root.path) {
        fs::remove_file(root.path.join(name)).expect("可以清空原根");
    }
    fs::remove_dir(&root.path).expect("可以删掉原根");
    fs::create_dir(&root.path).expect("可以建同名新根");
    fs::set_permissions(&root.path, fs::Permissions::from_mode(0o700)).expect("可以设私有权限");

    assert_eq!(publisher.check().err(), Some(TransportError::UnsafeStorage));
    assert_eq!(publisher.read().err(), Some(TransportError::UnsafeStorage));
    assert_eq!(
        publisher.replace(&records.second).err(),
        Some(TransportError::UnsafeStorage)
    );
    assert!(names(&root.path).is_empty());
}
