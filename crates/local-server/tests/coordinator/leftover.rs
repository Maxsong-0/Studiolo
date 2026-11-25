//! 死 socket 接管的判断：凭据、探活、类型、二次核各自单独挡得住；挡住时节点一个不少。
use super::fixture::Fixture;
use crate::unix_ingress::leftover::{Entrance, Refused};
use std::{
    fs::{self, File},
    os::{
        fd::AsFd as _,
        unix::{
            fs::MetadataExt as _,
            net::{UnixDatagram, UnixListener},
        },
    },
    path::{Path, PathBuf},
};

/// 放入口的目录：自己开着父目录，路径短到能当 socket 名字。
struct Run {
    temporary: tempfile::TempDir,
    parent: File,
}

impl Run {
    fn new() -> Self {
        let temporary = tempfile::Builder::new()
            .prefix("stu-left-")
            .tempdir_in("/tmp")
            .unwrap();
        let parent = File::open(temporary.path()).unwrap();
        Self { temporary, parent }
    }

    fn path(&self) -> PathBuf {
        self.temporary.path().join("entrance.sock")
    }

    fn dead(&self) -> u64 {
        drop(UnixListener::bind(self.path()).unwrap());
        inode(&self.path())
    }

    /// 交给 `check` 一个入口；句柄只在这期间借着。
    fn entrance<T>(&self, check: impl FnOnce(&Entrance<'_>) -> T) -> T {
        let path = self.path();
        check(&Entrance {
            parent: self.parent.as_fd(),
            name: path.file_name().unwrap(),
            path: &path,
            label: "测试入口",
        })
    }
}

fn inode(path: &Path) -> u64 {
    fs::symlink_metadata(path).unwrap().ino()
}

#[tokio::test]
async fn a_dead_socket_is_removed_once_and_an_empty_place_needs_no_credential() {
    let f = Fixture::new();
    let workbench = f.start(false).await;
    let run = Run::new();
    run.dead();
    run.entrance(|entrance| {
        entrance.acceptable().unwrap();
        assert!(run.path().exists(), "只看不动");
        entrance.take_over(&workbench).unwrap();
        assert!(!run.path().exists());
        entrance.take_over(&workbench).unwrap();
    });
    workbench.shutdown().await.unwrap();
    run.entrance(|entrance| entrance.take_over(&workbench))
        .unwrap();
}

#[tokio::test]
async fn a_writer_that_is_closing_or_has_handed_its_broker_over_cannot_take_over() {
    let f = Fixture::new();
    let workbench = f.start(false).await;
    let run = Run::new();
    let left = run.dead();
    // 封存的冻结窗口里 Broker 不在这里：证明不了锁还在自己手上。
    let broker = workbench.core.owner.lock().unwrap().take();
    assert_eq!(
        run.entrance(|entrance| entrance.take_over(&workbench)),
        Err(Refused)
    );
    *workbench.core.owner.lock().unwrap() = broker;
    // 已经在关、Broker 还没收走的那一小段也不行。
    workbench.core.control.lock().unwrap().closing = true;
    assert!(workbench.sole_instance().is_err());
    assert_eq!(
        run.entrance(|entrance| entrance.take_over(&workbench)),
        Err(Refused)
    );
    workbench.core.control.lock().unwrap().closing = false;
    assert!(workbench.sole_instance().is_ok());
    workbench.shutdown().await.unwrap();
    assert_eq!(
        run.entrance(|entrance| entrance.take_over(&workbench)),
        Err(Refused)
    );
    assert_eq!(inode(&run.path()), left);
}

#[tokio::test]
async fn a_changed_approved_root_takes_the_credential_away() {
    let f = Fixture::new();
    let workbench = f.start(false).await;
    let run = Run::new();
    let left = run.dead();
    fs::rename(f.root.join("assets"), f.root.join("assets-old")).unwrap();
    fs::create_dir(f.root.join("assets")).unwrap();
    assert_eq!(
        run.entrance(|entrance| entrance.take_over(&workbench)),
        Err(Refused)
    );
    assert_eq!(inode(&run.path()), left);
    workbench.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_socket_someone_hears_or_that_is_not_a_stream_is_kept() {
    let f = Fixture::new();
    let workbench = f.start(false).await;
    let run = Run::new();
    let listener = UnixListener::bind(run.path()).unwrap();
    let live = inode(&run.path());
    run.entrance(|entrance| {
        assert_eq!(entrance.acceptable(), Err(Refused));
        assert_eq!(entrance.take_over(&workbench), Err(Refused));
    });
    assert_eq!(inode(&run.path()), live);
    drop(listener);
    fs::remove_file(run.path()).unwrap();
    // 数据报 socket 不是这里建的那一种：连它回的是类型不符，不是没人。
    let _datagram = UnixDatagram::bind(run.path()).unwrap();
    assert_eq!(
        run.entrance(|entrance| entrance.take_over(&workbench)),
        Err(Refused)
    );
    assert!(run.path().exists());
    workbench.shutdown().await.unwrap();
}

/// 探活说没人之后、删之前，名字换了主人：那个结论不是说它的，新来的一个不许删。
#[tokio::test]
async fn a_node_swapped_between_the_probe_and_the_removal_is_kept() {
    let f = Fixture::new();
    let workbench = f.start(false).await;
    for same_inode in [false, true] {
        let run = Run::new();
        run.dead();
        let sole = workbench.sole_instance().unwrap();
        let mut arrived = None;
        let refused = run.entrance(|entrance| {
            entrance.retire_leftover(&sole, |path| {
                if same_inode {
                    // 同一个 inode，但被人改过：多一个名字又去掉，只有 ctime 看得出来。
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let other = path.with_extension("other");
                    fs::hard_link(path, &other).unwrap();
                    fs::remove_file(other).unwrap();
                } else {
                    let replacement = path.with_extension("new");
                    arrived = Some(UnixListener::bind(&replacement).unwrap());
                    fs::rename(replacement, path).unwrap();
                }
                true
            })
        });
        assert!(refused.is_err(), "same_inode={same_inode}");
        assert!(run.path().exists(), "same_inode={same_inode}");
        drop(arrived);
    }
    workbench.shutdown().await.unwrap();
}

/// 路径指到了别的目录：连的是那边的死 socket，手里这边的是活的——两边对不上就不动，哪边都不删。
#[tokio::test]
async fn a_parent_swapped_under_the_path_is_not_judged_by_the_other_directory() {
    let f = Fixture::new();
    let workbench = f.start(false).await;
    let run = Run::new();
    let _alive = UnixListener::bind(run.path()).unwrap();
    let moved = run.temporary.path().with_extension("moved");
    fs::rename(run.temporary.path(), &moved).unwrap();
    fs::create_dir(run.temporary.path()).unwrap();
    drop(UnixListener::bind(run.path()).unwrap());
    assert_eq!(
        run.entrance(|entrance| entrance.take_over(&workbench)),
        Err(Refused)
    );
    assert!(run.path().exists());
    assert!(moved.join("entrance.sock").exists());
    fs::remove_dir_all(moved).unwrap();
    workbench.shutdown().await.unwrap();
}

/// 借着凭据的时候主库锁放不开：另一任这时起不来。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_credential_keeps_the_writer_lock_until_it_is_returned() {
    let f = Fixture::new();
    let workbench = f.start(false).await;
    let open = || {
        studiolo_vault_broker::FsVaultBroker::open_existing(
            &f.root.join("vault"),
            &f.root.join("data"),
        )
    };
    {
        let _sole = workbench.sole_instance().unwrap();
        workbench.stop();
        // 关停在别的线程上跑，给它足够的时间放锁：凭据没还，它就放不掉。
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(matches!(
            open(),
            Err(studiolo_vault_broker::BrokerError::WriterBusy)
        ));
    }
    workbench.shutdown().await.unwrap();
    f.available();
}
