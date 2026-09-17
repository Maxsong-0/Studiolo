//! 独立进程实写大对象；只使用有界摘要检查，不借小文件试验声明跨路径能力。
use super::*;
use std::io::Write;
use studiolo_core::RevisionHasher;

const BYTES: usize = 200 * 1024 * 1024 + 17;
const RAW: &str = "1606f5a7d61b07583745b90864f05b332d60140b876c93025d07e1b1baaa13db";
const CHILD: &str = "file_operation::tests::rename::large::large_child";

#[test]
fn large_rename_projection_is_measured_in_an_isolated_process() {
    // 与既有附件/删除恢复测试共用父进程锁，避免同时堆叠多份大对象。
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
            CHILD,
            "--ignored",
            "--test-threads=1",
            "--nocapture",
        ])
        .env("LC_ALL", "C")
        .env("STUDIOLO_PRIVATE_RENAME_MEMORY_CHILD", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    // 精确模块名失配时 libtest 仍可成功退出；零测试不能冒充测量通过。
    assert!(stdout.contains(CHILD), "{stdout}\n{stderr}");
    assert!(stdout.contains("1 passed; 0 failed"), "{stdout}\n{stderr}");
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
#[ignore = "仅由受控父测试测量；真实大文件由子进程 TempDir 在正常退出或展开时清理"]
fn large_child() {
    assert_eq!(
        std::env::var("STUDIOLO_PRIVATE_RENAME_MEMORY_CHILD").unwrap(),
        "1"
    );
    let _guard = crate::blob::LARGE_IO.lock().unwrap();
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let from = path();
    let to = logical("renamed/payload.md");
    fs::create_dir(fixture.root.join("renamed")).unwrap();
    let (source_revision, target_revision) = write_pattern(&fixture, &broker, &from, &to);
    assert_ne!(source_revision, target_revision);
    let before = crate::blob::file::scan(
        &broker.root,
        &from,
        crate::blob::OBJECT_LIMIT,
        Some((broker.vault_uuid, &from)),
    )
    .unwrap();
    assert_eq!(before.identity.bytes, BYTES as u64);
    assert_eq!(before.identity.sha256, RAW);
    assert_eq!(before.revision, Some(source_revision));
    broker.enable_file_operations(limits()).unwrap();
    broker.enable_file_rename().unwrap();
    let original = request(from.as_str(), to.as_str(), source_revision, "large-rename");
    let ticket = broker
        .prepare_file_operation(original.clone(), metadata())
        .unwrap();
    let active = crate::mutation::load(&broker)
        .unwrap()
        .file_operation
        .unwrap();
    assert_eq!(active.phase, FileOperationPhase::Prepared);
    let stage = active.sealed_stage.as_ref().unwrap();
    let projected = active.rename.as_ref().unwrap().history.as_ref().unwrap();
    assert_eq!(
        active.history.as_ref().unwrap().revision,
        Some(source_revision)
    );
    assert_eq!(active.history.as_ref().unwrap().identity, before.identity);
    assert_eq!(projected.revision, Some(target_revision));
    assert_eq!(projected.identity, before.identity);
    assert_eq!(stage.revision, Some(target_revision));
    assert_eq!(stage.identity, before.identity);
    intent(&mut broker, &ticket);
    let result = broker.commit_file_operation(&ticket).unwrap();
    assert_eq!(result.outcome, FileOperationOutcome::Committed);
    assert_eq!(result.sequence, 1);
    assert_eq!(
        result.effects,
        vec![
            FileEffect::Confirmed {
                path: to.clone(),
                before: FilePresence::Missing,
                after: present(target_revision),
            },
            FileEffect::Confirmed {
                path: from.clone(),
                before: present(source_revision),
                after: FilePresence::Missing,
            },
        ]
    );
    assert!(
        broker
            .file_operation_status()
            .unwrap()
            .unwrap()
            .audit_pending
    );
    flush(&mut broker);
    assert!(broker.file_operation_status().unwrap().is_none());
    assert!(!fixture.root.join(from.as_str()).exists());
    let target = crate::blob::file::scan(
        &broker.root,
        &to,
        crate::blob::OBJECT_LIMIT,
        Some((broker.vault_uuid, &to)),
    )
    .unwrap();
    assert_eq!(target.identity, before.identity);
    assert_eq!(target.revision, Some(target_revision));
    // 选择真正已发布 catalog，再以流式核验两份路径绑定 L0；不调用大 Vec 正文接口。
    let source_page = broker.list_history(&from, 10, None).unwrap();
    let target_page = broker.list_history(&to, 10, None).unwrap();
    assert_eq!(source_page.entries.len(), 1);
    assert_eq!(target_page.entries.len(), 1);
    assert_ne!(source_page.entries[0].id, target_page.entries[0].id);
    for (path, revision, page) in [
        (&from, source_revision, &source_page),
        (&to, target_revision, &target_page),
    ] {
        assert_eq!(page.entries[0].revision, revision);
        assert_eq!(page.entries[0].bytes, BYTES as u64);
        assert!(page.entries[0].author.is_none());
        assert_eq!(page.entries[0].archived_by, Some(metadata().actor));
        assert_eq!(
            crate::history::blob::selected(&broker, path, revision).unwrap(),
            before.identity
        );
    }
    let tombstones = broker.list_tombstones(10, None).unwrap();
    assert_eq!(tombstones.entries.len(), 1);
    let tombstone = &tombstones.entries[0];
    assert_eq!(Some(&tombstone.id), result.tombstone_id.as_ref());
    assert_eq!(tombstone.path, from);
    assert_eq!(tombstone.revision, source_revision);
    assert_eq!(tombstone.sha256, RAW);
    assert_eq!(tombstone.bytes, BYTES as u64);
    if let (Some(start), Some(end)) = (tombstone.deleted_at_ms, tombstone.retain_until_ms) {
        assert!(end - start >= 7 * 86_400_000);
    }
    let quarantine = crate::blob::store::verify(
        &broker,
        &ops::store::required(&broker).unwrap().quarantine,
        &format!("{}.object", result.operation_id),
        &from,
        source_revision,
    )
    .unwrap();
    assert_eq!(quarantine.identity, before.identity);
    assert_eq!(quarantine.node.file.identity, before.node.file.identity);
    drop(broker);
    let mut broker = fixture.reopen();
    assert_eq!(receipt(&broker, &original), result);
    let replay = broker
        .prepare_file_operation(original.clone(), metadata())
        .unwrap();
    assert_eq!(broker.commit_file_operation(&replay).unwrap(), result);
    assert!(!fixture.root.join(from.as_str()).exists());
    // 同一目标文件完整 Stamp 未变，原键不能再安装或重写另一个大对象。
    crate::blob::file::check(&broker.root, &to, &target.node).unwrap();
    assert_eq!(broker.list_history(&from, 10, None).unwrap(), source_page);
    assert_eq!(broker.list_history(&to, 10, None).unwrap(), target_page);
    assert_eq!(broker.list_tombstones(10, None).unwrap(), tombstones);
    assert_eq!(broker.write_status().unwrap().committed_through, 1);
    assert!(!broker.write_status().unwrap().unresolved);
    assert_eq!(crate::mutation::load(&broker).unwrap().next_sequence, 2);
}

fn present(revision: Revision) -> FilePresence {
    FilePresence::Present {
        revision,
        sha256: RAW.to_owned(),
        bytes: BYTES as u64,
    }
}

fn write_pattern(
    fixture: &Fixture,
    broker: &FsVaultBroker,
    from: &VaultPath,
    to: &VaultPath,
) -> (Revision, Revision) {
    let mut source = RevisionHasher::new(broker.vault_uuid, from);
    let mut target = RevisionHasher::new(broker.vault_uuid, to);
    let chunk: Vec<u8> = (0..65536).map(|i| ((i * 17 + 3) % 251) as u8).collect();
    let mut output = fs::File::create(fixture.root.join(from.as_str())).unwrap();
    let mut remaining = BYTES;
    while remaining > 0 {
        let bytes = &chunk[..remaining.min(chunk.len())];
        output.write_all(bytes).unwrap();
        source.update(bytes);
        target.update(bytes);
        remaining -= bytes.len();
    }
    output.sync_all().unwrap();
    (source.finish(), target.finish())
}
