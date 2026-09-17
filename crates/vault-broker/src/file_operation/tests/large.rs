use super::*;
use std::io::Write;
#[test]
fn large_delete_restore_is_measured_in_an_isolated_process() {
    let _guard = crate::blob::LARGE_IO.lock().unwrap();
    let mut command = std::process::Command::new("/usr/bin/time");
    #[cfg(target_os = "macos")]
    command.arg("-l");
    #[cfg(target_os = "linux")]
    command.arg("-v");
    let output = command
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "file_operation::tests::large::large_child",
            "--ignored",
            "--test-threads=1",
            "--nocapture",
        ])
        .env("LC_ALL", "C")
        .env("STUDIOLO_PRIVATE_FILE_OPERATION_CHILD", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{}\n{stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    #[cfg(target_os = "macos")]
    let peak = stderr
        .lines()
        .find(|line| line.contains("maximum resident set size"))
        .and_then(|line| line.split_whitespace().next())
        .unwrap()
        .parse::<u64>()
        .unwrap();
    #[cfg(target_os = "linux")]
    let peak = stderr
        .lines()
        .find(|line| line.contains("Maximum resident set size (kbytes)"))
        .and_then(|line| line.rsplit(':').next())
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap()
        * 1024;
    assert!(peak < 128 * 1024 * 1024, "measured peak RSS: {peak}");
}
#[test]
#[ignore = "只由受控父测试启动，TempDir 在子进程正常退出时清理"]
fn large_child() {
    assert_eq!(
        std::env::var("STUDIOLO_PRIVATE_FILE_OPERATION_CHILD").unwrap(),
        "1"
    );
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let bytes = 200 * 1024 * 1024 + 17;
    let chunk: Vec<u8> = (0..65536).map(|i| ((i * 17 + 3) % 251) as u8).collect();
    let mut file = fs::File::create(fixture.root.join("note.md")).unwrap();
    for _ in 0..bytes / chunk.len() {
        file.write_all(&chunk).unwrap();
    }
    file.write_all(&chunk[..bytes % chunk.len()]).unwrap();
    file.sync_all().unwrap();
    drop(file);
    broker.enable_file_operations(limits()).unwrap();
    let before = crate::blob::file::scan(
        &broker.root,
        &path(),
        crate::blob::OBJECT_LIMIT,
        Some((broker.vault_uuid, &path())),
    )
    .unwrap();
    assert_eq!(before.identity.bytes, bytes as u64);
    let deleted = committed(
        &mut broker,
        delete(before.revision.unwrap(), "large-delete"),
    );
    committed(
        &mut broker,
        restore(deleted.tombstone_id.as_deref().unwrap(), "large-restore"),
    );
    let after = crate::blob::file::scan(
        &broker.root,
        &path(),
        crate::blob::OBJECT_LIMIT,
        Some((broker.vault_uuid, &path())),
    )
    .unwrap();
    assert_eq!(after.identity, before.identity);
    assert_eq!(after.revision, before.revision);
}
