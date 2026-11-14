//! 真进程死亡不运行 Drop；正文仍由未完成标记隔离，不能因内核锁释放而继续备份。
use super::{fixture::*, *};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};
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
