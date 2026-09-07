//! Linux-only kernel path and nonblocking race checks; not macOS evidence.

use rustix::fs::{CWD, Mode, RenameFlags, mkfifoat, renameat_with};
use std::{
    fs,
    os::{fd::AsRawFd as _, unix::fs::symlink},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use studiolo_vault_broker::BrokerError;

use super::support::{Fixture, path};

#[test]
fn proc_magic_link_targets_are_refused_by_the_canonical_handle() {
    let fixture = Fixture::new();
    let outside = fixture.directory.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret.md"), b"outside secret").unwrap();
    let open_directory = fs::File::open(&outside).unwrap();
    symlink(
        format!("/proc/self/fd/{}", open_directory.as_raw_fd()),
        fixture.root.join("magic"),
    )
    .unwrap();
    assert!(matches!(
        fixture.reader().read_file(&path("magic/secret.md"), 64),
        Err(BrokerError::UnsafeNode { .. })
    ));
}

#[test]
fn racing_symlinks_hardlinks_and_fifos_never_escape_or_block_public_reads() {
    for attack in ["symlink", "hardlink", "fifo"] {
        let fixture = Fixture::new();
        let slot = fixture.root.join("note.md");
        let alternate = fixture.root.join("alternate");
        let secret = fixture.directory.path().join("secret.md");
        fs::write(&slot, b"canonical").unwrap();
        fs::write(&secret, b"outside secret").unwrap();
        match attack {
            "symlink" => symlink(&secret, &alternate).unwrap(),
            "hardlink" => fs::hard_link(&secret, &alternate).unwrap(),
            "fifo" => mkfifoat(CWD, &alternate, Mode::RUSR | Mode::WUSR).unwrap(),
            _ => unreachable!(),
        }
        let reader = fixture.reader();
        let stop = Arc::new(AtomicBool::new(false));
        let writer_stop = Arc::clone(&stop);
        let writer = thread::spawn(move || {
            while !writer_stop.load(Ordering::Acquire) {
                renameat_with(CWD, &slot, CWD, &alternate, RenameFlags::EXCHANGE).unwrap();
                thread::yield_now();
            }
        });
        let (sender, receiver) = mpsc::channel();
        let reading = thread::spawn(move || {
            let escaped = (0..256).any(|_| {
                reader
                    .read_file(&path("note.md"), 64)
                    .is_ok_and(|read| read.content != b"canonical")
            });
            sender.send(escaped).unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(15));
        stop.store(true, Ordering::Release);
        writer.join().unwrap();
        assert!(result.is_ok(), "{attack} caused a blocked canonical read");
        assert!(!result.unwrap(), "{attack} exposed outside content");
        reading.join().unwrap();
        assert_eq!(fs::read(secret).unwrap(), b"outside secret");
    }
}
