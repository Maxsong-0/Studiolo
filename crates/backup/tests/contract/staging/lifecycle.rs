//! 真进程死亡不运行 Drop；正文仍由未完成标记隔离，不能因内核锁释放而继续备份。
use super::{fixture::*, *};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};
use studiolo_backup::checkpoint::staging::reclaim_staging;
const CHILD: &str = "contract::staging::lifecycle::child_holds_original_area";
#[test]
#[ignore = "仅由生命周期父测试在虚构目录内启动"]
fn child_holds_original_area() {
    let root = std::env::var_os("STUDIOLO_STAGING_CHILD_ROOT").expect("生命周期子测试根");
    let root = std::path::Path::new(&root);
    let area = StagingArea::create(
        &root.join("staging"),
        checkpoint("crashed"),
        &[root.join("source")],
        &root.join("ephemeral"),
        deadline(),
    )
    .unwrap();
    private_file(&area.export_root().join("canonical/raw"), b"unsealed raw");
    println!("STAGING-READY");
    std::io::stdout().flush().unwrap();
    loop {
        std::thread::park();
    }
}
#[test]
fn abrupt_producer_death_keeps_the_persistent_gate_closed() {
    let f = Fixture::new();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--ignored", "--nocapture"])
        .env("STUDIOLO_STAGING_CHILD_ROOT", f.temp.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line.contains("STAGING-READY") {
            break;
        }
    }
    assert_eq!(f.try_area("while-alive").err(), Some(StagingError::Busy));
    assert_eq!(
        studiolo_backup::checkpoint::staging::reclaim_staging(&f.parent, None, deadline()),
        Err(StagingError::Busy)
    );
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        f.try_area("after-death").err(),
        Some(StagingError::CleanupRequired)
    );
    assert!(f.parent.join(".producer-lease").is_file());
    assert!(fs::read_dir(&f.parent).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("incomplete-")
    }));
    let marker = fs::read_dir(&f.parent)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("incomplete-")
        })
        .unwrap();
    // 已有原身份记录；崩溃留下的半写临时 JSON 不是完成回执。
    private_file(&marker.join("sealed.new"), b"{");
    let reclaimed =
        studiolo_backup::checkpoint::staging::reclaim_staging(&f.parent, None, deadline()).unwrap();
    assert_eq!(reclaimed.interrupted, 1);
    assert_eq!(fs::read_dir(&f.parent).unwrap().count(), 1);
    f.area("after-recovery").abort().unwrap();
}
#[test]
fn explicit_abort_releases_the_gate_only_after_both_original_directories_are_removed() {
    let f = Fixture::new();
    let area = f.area("first");
    let export = area.export_root().to_owned();
    private_file(&export.join("canonical/raw"), b"must be reclaimed");
    assert_eq!(f.try_area("second").err(), Some(StagingError::Busy));
    area.abort().unwrap();
    assert!(!export.exists());
    f.area("second").abort().unwrap();
    assert_eq!(fs::read_dir(&f.parent).unwrap().count(), 1);
}
#[test]
fn legacy_orphans_and_replaced_lock_fail_closed_without_deleting_them() {
    let f = Fixture::new();
    private_dir(&f.parent.join("export-legacy"));
    private_file(
        &f.parent.join("export-legacy/raw"),
        b"not registered by this attempt",
    );
    assert_eq!(f.try_area("new").err(), Some(StagingError::CleanupRequired));
    assert!(f.parent.join("export-legacy/raw").is_file());
    let f = Fixture::new();
    let area = f.area("original");
    fs::rename(
        f.parent.join(".producer-lease"),
        f.parent.join("original-lock"),
    )
    .unwrap();
    private_file(&f.parent.join(".producer-lease"), b"");
    assert_eq!(area.abort(), Err(StagingError::CleanupFailed));
    assert_eq!(f.try_area("new").err(), Some(StagingError::CleanupRequired));
}
/// 人侧备份生产者先回收、紧接着建暂存，同一进程里看板执行器可能正在起子进程（BUG-041）。
/// 子进程到 exec 之前握着持锁描述的副本，放下的租约必须立即可重拿；析构只关 FD 时，
/// 这里约六成轮次报 `Busy`（测试箱 x86_64 实测）。
#[test]
fn reclaim_then_create_never_sees_busy_while_other_threads_spawn_children() {
    const ROUNDS: usize = 400;
    let f = Fixture::new();
    f.area("seed").abort().unwrap();
    let stop = AtomicBool::new(false);
    let failures: Vec<StagingError> = std::thread::scope(|s| {
        for _ in 0..2 {
            s.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    Command::new("true").status().unwrap();
                }
            });
        }
        // 循环里不许 panic：先停掉起子进程的线程再断言，否则 scope 等不到它们退出。
        let failures = (0..ROUNDS)
            .filter_map(|_| {
                let reclaimed = reclaim_staging(&f.parent, None, deadline()).err();
                let created = f.try_area("round").and_then(StagingArea::abort).err();
                reclaimed.or(created)
            })
            .collect();
        stop.store(true, Ordering::Relaxed);
        failures
    });
    assert!(
        failures.is_empty(),
        "{ROUNDS} 轮里 {} 轮失败，前几个：{:?}",
        failures.len(),
        &failures[..failures.len().min(5)]
    );
}
