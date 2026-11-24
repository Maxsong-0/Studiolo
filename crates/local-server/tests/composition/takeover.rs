//! 人侧被强杀之后能自己再起来：三个入口留下的 socket 由程序判死才接管，判不准一律拒绝、一个不删。
//!
//! 2026-11-24 169 上人侧被 OOM 杀，systemd 自动重启 3 次都被残留 socket 拒绝，停站 31 分钟（T-231）。
//! 这里真起一个子进程跑完整装配、真 `kill -9`，不手工摆残留。
use super::super::{PreparedServer, prepare_composed};
use super::{
    Fixture, StartupError, backup_directories::backup_config, checkpoint::policy, test_support,
    writer::writer_options,
};
use crate::options::{BoardOptions, FactorOptions, Options};
use std::{
    fs,
    os::unix::{
        fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use studiolo_vault_broker::FsVaultBroker;

const CHILD: &str = "startup::tests::takeover::child_serves_every_entrance_until_killed";
const CHILD_ROOT: &str = "STUDIOLO_TAKEOVER_TEST_ROOT";
/// 子进程起到三个入口都在监听的上限；测试箱忙时装配本身就要十几秒。
const WAIT: Duration = Duration::from_secs(120);

/// 三个入口的路径，次序就是起服务时建立的次序。
fn sockets(root: &Path) -> [PathBuf; 3] {
    [
        root.join("bsock/p.sock"),
        root.join("board-ingress/board.sock"),
        root.join("proxy/http.sock"),
    ]
}

/// 建好照生产人侧装配要的目录和库；之后用 [`wire`] 指过去就能反复起。
fn lay_out(f: &Fixture) {
    let (mut options, _) = writer_options(f);
    policy(f, &mut options, true, true);
    let proxy = f.root.join("proxy");
    fs::create_dir(&proxy).unwrap();
    fs::set_permissions(&proxy, fs::Permissions::from_mode(0o700)).unwrap();
    backup_config(f);
}

/// 只按路径接线，不建任何东西：父进程和子进程用的是同一份启动参数。
fn wire(f: &Fixture) -> Options {
    let mut options = f.options();
    options.writer_data = Some(f.root.join("writer-data"));
    options.policy_data = Some(f.root.join("policy"));
    options.agent_data = Some(f.root.join("identities"));
    options.board = Some(BoardOptions {
        data: f.root.join("control"),
        agent_data: f.root.join("identities"),
        socket: f.root.join("board-ingress/board.sock"),
    });
    options.factor = Some(FactorOptions {
        socket: f.root.join("proxy/http.sock"),
        replacement_seed_ref: None,
    });
    options.backup_config = Some(f.root.join("config/backup.json"));
    options
}

async fn prepare(f: &Fixture) -> Result<PreparedServer, StartupError> {
    prepare_composed(
        wire(f),
        |_| async { Ok(test_support::verifier().await) },
        || {
            (
                Some("3132333435363738393031323334353637383930".into()),
                None,
            )
        },
    )
    .await
}

/// 起到三个入口都在监听，等 `stop`；回来时服务已经收掉。
async fn serves(f: &Fixture) -> Result<(), StartupError> {
    let prepared = prepare(f).await?;
    tokio::time::timeout(WAIT, prepared.serve_until(std::future::ready(())))
        .await
        .expect("服务没在时限内停下")
}

/// 一个 socket 现在是谁：换过 inode 或者重建过都看得出来。
#[derive(Debug, PartialEq, Eq)]
struct Seen(u64, u64, i64, i64);

fn seen(path: &Path) -> Seen {
    let meta = fs::symlink_metadata(path).unwrap();
    Seen(meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec())
}

/// 留下一个没人监听的 socket：监听关掉，名字还在，和进程被强杀留下的一样。
fn dead(path: &Path) {
    drop(UnixListener::bind(path).unwrap());
    assert!(fs::symlink_metadata(path).unwrap().file_type().is_socket());
}

struct OwnedChild(Child);

impl OwnedChild {
    /// 起子进程；标准错误落到 `log`，里面有接管时打的行。
    fn start(f: &Fixture, log: &Path) -> Self {
        Self(
            Command::new(std::env::current_exe().unwrap())
                .args(["--ignored", "--exact", CHILD, "--nocapture"])
                .env(CHILD_ROOT, &f.root)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(fs::File::create(log).unwrap())
                .spawn()
                .unwrap(),
        )
    }

    /// 三个入口都连得上才算起来了；残留的死 socket 连不上，不会误判。
    fn wait_listening(&mut self, f: &Fixture, log: &Path) {
        let limit = Instant::now() + WAIT;
        for path in sockets(&f.root) {
            while UnixStream::connect(&path).is_err() {
                let exited = self.0.try_wait().unwrap();
                assert!(
                    exited.is_none() && Instant::now() < limit,
                    "人侧没起来：{path:?}，退出 {exited:?}，日志：{}",
                    fs::read_to_string(log).unwrap()
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    /// 真的强杀：不给它任何收尾的机会。
    fn kill(mut self) {
        self.0.kill().unwrap();
        assert!(!self.0.wait().unwrap().success());
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "只由下面的父测试当子进程拉起"]
async fn child_serves_every_entrance_until_killed() {
    let f = Fixture::reopened(PathBuf::from(std::env::var_os(CHILD_ROOT).unwrap()));
    prepare(&f)
        .await
        .unwrap()
        .serve_until(std::future::pending())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_killed_server_starts_again_on_its_own_and_says_what_it_took_over() {
    let f = Fixture::in_tmp(true);
    lay_out(&f);
    let first_log = f.root.join("first.log");
    let mut first = OwnedChild::start(&f, &first_log);
    first.wait_listening(&f, &first_log);
    assert!(
        !fs::read_to_string(&first_log).unwrap().contains("接管"),
        "没有残留就不该说接管"
    );
    first.kill();
    let left: Vec<Seen> = sockets(&f.root).iter().map(|path| seen(path)).collect();

    let second_log = f.root.join("second.log");
    let mut second = OwnedChild::start(&f, &second_log);
    second.wait_listening(&f, &second_log);
    let log = fs::read_to_string(&second_log).unwrap();
    let lines: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("接管上一任留下的"))
        .collect();
    assert_eq!(lines.len(), 3, "每个入口各说一行：{log}");
    for ((line, name), left) in lines
        .iter()
        .zip(["备份入口", "黑板入口", "HTTP 入口"])
        .zip(&left)
    {
        assert!(line.contains(name), "{line}");
        assert!(line.contains(&format!("inode {}", left.1)), "{line}");
        assert!(
            !line.contains(f.root.to_str().unwrap()),
            "日志不带路径：{line}"
        );
    }
    let alive: Vec<Seen> = sockets(&f.root).iter().map(|path| seen(path)).collect();
    for (path, mode) in sockets(&f.root).iter().zip([0o660, 0o600, 0o600]) {
        assert_eq!(
            fs::symlink_metadata(path).unwrap().mode() & 0o7777,
            mode,
            "接管之后的权限和平时一样：{path:?}"
        );
    }

    // 另一任活着：再起一个只会被拒绝，活着的那一任的三个 socket 一个不少、照常接连接。
    assert!(prepare(&f).await.is_err());
    for (path, before) in sockets(&f.root).iter().zip(&alive) {
        assert_eq!(&seen(path), before);
        UnixStream::connect(path).unwrap();
    }
    second.kill();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leftovers_of_all_three_entrances_are_replaced_and_a_clean_stop_leaves_none() {
    let f = Fixture::in_tmp(true);
    lay_out(&f);
    for path in sockets(&f.root) {
        dead(&path);
    }
    serves(&f).await.unwrap();
    for path in sockets(&f.root) {
        assert!(!path.exists(), "正常停服照旧自己删：{path:?}");
    }
    f.unbound();
}

/// 主库锁在别人手里时，就算三个都连不上（对方还没开始监听），也一个不碰。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_writer_lock_keeps_every_leftover() {
    let f = Fixture::in_tmp(true);
    lay_out(&f);
    for path in sockets(&f.root) {
        dead(&path);
    }
    let before: Vec<Seen> = sockets(&f.root).iter().map(|path| seen(path)).collect();
    let owner =
        FsVaultBroker::open_existing(&f.root.join("vault"), &f.root.join("writer-data")).unwrap();
    assert!(matches!(prepare(&f).await, Err(StartupError::Writer)));
    for (path, before) in sockets(&f.root).iter().zip(&before) {
        assert_eq!(&seen(path), before, "{path:?}");
    }
    drop(owner);
    serves(&f).await.unwrap();
}

/// 每个入口各自占住一次：活着的监听、监听队列满、普通文件、目录、链接、管道、两个名字都拒绝，且不删。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anything_that_is_not_a_dead_socket_of_ours_is_refused_and_kept() {
    let f = Fixture::in_tmp(true);
    lay_out(&f);
    for path in sockets(&f.root) {
        for kind in [
            "live",
            "full",
            "file",
            "directory",
            "link",
            "fifo",
            "two-names",
        ] {
            let held = occupy(&f, &path, kind);
            let before = seen(&path);
            assert!(
                matches!(serves(&f).await, Err(StartupError::Ingress)),
                "{path:?} {kind}"
            );
            assert_eq!(seen(&path), before, "{path:?} {kind}");
            drop(held);
            if kind == "directory" {
                fs::remove_dir(&path).unwrap();
            } else {
                fs::remove_file(&path).unwrap();
            }
            let _ = fs::remove_file(f.root.join("elsewhere.sock"));
            // 别的入口没留下东西：先建好的那几个照旧自己收走。
            for other in sockets(&f.root) {
                assert!(!other.exists(), "{path:?} {kind} {other:?}");
            }
        }
    }
    serves(&f).await.unwrap();
}

/// 占住一个入口的名字；要一直开着的东西交回去拿着。
fn occupy(f: &Fixture, path: &Path, kind: &str) -> Vec<std::os::fd::OwnedFd> {
    let elsewhere = f.root.join("elsewhere.sock");
    match kind {
        "live" => vec![UnixListener::bind(path).unwrap().into()],
        "full" => {
            let listener = UnixListener::bind(path).unwrap();
            // 把排队长度压到最小再塞满：不等待的连接这时回 EAGAIN，不是 ECONNREFUSED。
            rustix::net::listen(&listener, 0).unwrap();
            let mut held: Vec<std::os::fd::OwnedFd> =
                vec![UnixStream::connect(path).unwrap().into()];
            held.push(listener.into());
            held
        }
        "file" => {
            fs::write(path, b"keep").unwrap();
            Vec::new()
        }
        "directory" => {
            fs::create_dir(path).unwrap();
            Vec::new()
        }
        "link" => {
            dead(&elsewhere);
            std::os::unix::fs::symlink(&elsewhere, path).unwrap();
            Vec::new()
        }
        "fifo" => {
            assert!(Command::new("mkfifo").arg(path).status().unwrap().success());
            Vec::new()
        }
        "two-names" => {
            dead(path);
            fs::hard_link(path, &elsewhere).unwrap();
            Vec::new()
        }
        other => unreachable!("{other}"),
    }
}

/// 没有写者就没有主库锁，证明不了自己是唯一一任：残留照旧拒绝。
#[tokio::test]
async fn without_a_writer_a_leftover_is_still_refused() {
    let f = Fixture::in_tmp(true);
    lay_out(&f);
    let path = f.root.join("proxy/http.sock");
    dead(&path);
    let before = seen(&path);
    for canonical in [false, true] {
        let mut options = f.options();
        options.canonical = canonical;
        options.factor = Some(FactorOptions {
            socket: path.clone(),
            replacement_seed_ref: None,
        });
        let result = prepare_composed(
            options,
            |_| async { panic!("入口不合格不得取钥") },
            || panic!("入口不合格不得读取秘密槽"),
        )
        .await;
        assert!(matches!(result, Err(StartupError::Ingress)));
        assert_eq!(seen(&path), before);
    }
}
