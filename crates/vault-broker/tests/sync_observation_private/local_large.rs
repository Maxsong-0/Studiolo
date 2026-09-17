//! 两种能力共用真实流式大文件；独立子进程量 RSS，不以小对象推断内存上限。
use super::*;
use crate::ReadOnlyVault;
use sha2::{Digest, Sha256};
use std::io::Write;

const BYTES: usize = 200 * 1024 * 1024 + 17;
const CHILD: &str = "sync_observation::tests::local_large::large_child";

#[test]
fn canonical_and_local_large_facts_have_measured_bounded_rss() {
    let _serial = crate::blob::LARGE_IO.lock().unwrap();
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
        .env("STUDIOLO_PRIVATE_LOCAL_OBSERVATION_CHILD", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
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
    println!("local observation peak RSS bytes: {peak}");
    assert!(peak < 128 * 1024 * 1024, "measured peak RSS: {peak}");
}

#[test]
#[ignore = "仅由父测试运行，实际 200MiB 文件与子进程内存均有独立证据"]
fn large_child() {
    assert_eq!(
        std::env::var("STUDIOLO_PRIVATE_LOCAL_OBSERVATION_CHILD").unwrap(),
        "1"
    );
    let _serial = crate::blob::LARGE_IO.lock().unwrap();
    let f = Fixture::new();
    let mut broker = f.broker();
    fs::create_dir(f.root.join("notes")).unwrap();
    let mut output = fs::File::create(f.root.join("notes/large")).unwrap();
    let chunk = [b'a'; 65536];
    let mut expected = Sha256::new();
    let mut remaining = BYTES;
    while remaining > 0 {
        let bytes = &chunk[..remaining.min(chunk.len())];
        output.write_all(bytes).unwrap();
        expected.update(bytes);
        remaining -= bytes.len();
    }
    output.sync_all().unwrap();
    drop(output);
    let hash = crate::hex(&expected.finalize());
    let reader = ReadOnlyVault::open(&f.root).unwrap();
    reset();
    let canonical = broker.begin_sync_observation(&selection(), &[]).unwrap();
    let local = reader.begin_scoped_observation(&selection(), &[]).unwrap();
    assert_eq!((count(1), count(3)), (0, 0));
    let canonical = broker.finish_sync_observation(canonical, &[]).unwrap();
    let local = reader.finish_scoped_observation(local, &[]).unwrap();
    let canonical_file = present(&canonical, "notes/large");
    let crate::LocalReadPresence::Present(local_file) = &local.entries()[&path("notes/large")]
    else {
        panic!("大文件必须完整观察");
    };
    assert_eq!(canonical_file.identity, local_file.identity);
    assert_eq!(local_file.identity.bytes, BYTES as u64);
    assert_eq!(local_file.identity.sha256, hash);
    assert!(local_file.utf8_valid && canonical_file.utf8_valid);
    assert_eq!(count(1), 2);
    assert_eq!(count(3), 2 * BYTES.div_ceil(65536));
    broker.revalidate_sync_observation(&canonical).unwrap();
    reader.revalidate_scoped_observation(&local).unwrap();
    assert_eq!(
        count(3),
        2 * BYTES.div_ceil(65536),
        "最终重验不得重新读取正文"
    );
}
