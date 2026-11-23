//! 文件锁放下必须显式解锁（BUG-041；同类前科 2026-09-28 的 `plane.lock`）。
//!
//! `flock` 的锁跟着打开的文件描述走。同进程别的线程起子进程时，子进程到 exec 之前握着每个描述的
//! 副本（`CLOEXEC` 到 exec 才关）。持锁的类型放下时只关自己的 FD，锁会活到子进程 exec，
//! 紧接着重拿就报 Busy：备份封存因此偶发失败，测试二进制里也偶发红。
//!
//! 规则按记号计数（注释、字符串不算）：每个源文件里每处加锁——`flock(…)` 参数里没有 `Unlock`
//! 的调用，以及 std 文件锁 `file.lock()`、`file.try_lock()`、`.lock_shared()`、`.try_lock_shared()`——
//! 都要对上一个 `impl Drop` 块，块里显式解锁（`flock(…Unlock)` 或 `.unlock()`）。
//! 守卫类型的 Drop 写在另一个文件时，在 [`COMPANIONS`] 里登记。认不出的写法：std 的
//! `lock()`／`try_lock()` 与 Mutex 同名，只认接收者叫 `file` 的调用，所以 std 文件锁的句柄变量叫 `file`。
//! `src/` 下 `tests` 目录和 `*tests.rs` 里是测试夹具，手动加解锁，不算。

use crate::support::{
    crate_directories, is_test_fixture,
    rust_lexer::{block, enclosed, lex},
    rust_sources, workspace_root,
};
use std::{fs, path::Path};

/// 加锁在前一个文件，包住锁的守卫类型（带析构解锁）在后一个文件。
const COMPANIONS: &[(&str, &str)] = &[(
    // `try_operation_lease` 把锁交给 `operation::Lease`，它的 Drop 显式解锁。
    "crates/audit-ledger/src/sqlite/storage/operation.rs",
    "crates/audit-ledger/src/operation.rs",
)];

#[derive(Debug, Default, PartialEq, Eq)]
struct Locks {
    /// 加锁调用处数。
    acquired: usize,
    /// 析构里显式解锁的 `impl Drop` 块数。
    released_on_drop: usize,
}

fn count(source: &str) -> Locks {
    let tokens = lex(source);
    let mut locks = Locks::default();
    for (index, token) in tokens.iter().enumerate() {
        let at = |offset: usize| tokens.get(index + offset).map(String::as_str);
        if token == "flock" && at(1) == Some("(") && !unlocks_call(&tokens[index + 1..]) {
            locks.acquired += 1;
        }
        if token == "." && at(2) == Some("(") {
            let receiver = index.checked_sub(1).map(|before| tokens[before].as_str());
            match at(1) {
                Some("lock_shared" | "try_lock_shared") => locks.acquired += 1,
                Some("lock" | "try_lock") if receiver == Some("file") => locks.acquired += 1,
                _ => {}
            }
        }
        let drop_unlocks =
            token == "impl" && is_drop_impl(&tokens[index..]) && releases(block(&tokens[index..]));
        if drop_unlocks {
            locks.released_on_drop += 1;
        }
    }
    locks
}

/// 从 `(` 起到配对的 `)` 为止的参数里有没有 `Unlock`。
fn unlocks_call(tokens: &[String]) -> bool {
    enclosed(tokens, "(", ")")
        .iter()
        .any(|token| token == "Unlock")
}

/// `impl … {` 的头部里有 `Drop for`。
fn is_drop_impl(tokens: &[String]) -> bool {
    let header = tokens.iter().take_while(|token| *token != "{");
    let header = header.map(String::as_str).collect::<Vec<_>>();
    header.windows(2).any(|pair| pair == ["Drop", "for"])
}

/// 块体里有显式解锁：`flock(…Unlock…)` 或 `.unlock()`。
fn releases(body: &[String]) -> bool {
    body.iter().enumerate().any(|(index, token)| {
        let at = |offset: usize| body.get(index + offset).map(String::as_str);
        (token == "flock" && at(1) == Some("(") && unlocks_call(&body[index + 1..]))
            || (token == "." && at(1) == Some("unlock") && at(2) == Some("("))
    })
}

fn read(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative)).expect("登记的配对文件必须可读")
}

#[test]
fn every_file_lock_is_handed_to_a_guard_that_unlocks_on_drop() {
    let root = workspace_root();
    let (mut violations, mut total) = (Vec::new(), 0);
    for directory in crate_directories(&root) {
        for path in rust_sources(&directory.join("src")) {
            let relative = path.strip_prefix(&root).expect("源码在工作区里");
            if is_test_fixture(relative) {
                continue;
            }
            let locks = count(&fs::read_to_string(&path).expect("源码必须可读"));
            if locks.acquired == 0 {
                continue;
            }
            total += locks.acquired;
            let name = relative.to_string_lossy().replace('\\', "/");
            let companion: usize = COMPANIONS
                .iter()
                .filter(|(locking, _)| *locking == name)
                .map(|(_, guard)| count(&read(&root, guard)).released_on_drop)
                .sum();
            let released = locks.released_on_drop + companion;
            if released < locks.acquired {
                violations.push(format!(
                    "{name}：{} 处加锁，只有 {released} 个析构里显式解锁的 Drop",
                    locks.acquired
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "文件锁放下时只关 FD 不够：并发起的子进程在 exec 之前握着描述副本，锁会多活到那时，紧接着重拿就报 Busy。\
         把锁交给一个守卫类型，在它的 `impl Drop` 里显式 `flock(…, FlockOperation::Unlock)`（std 文件锁用 `.unlock()`）；\
         Drop 在别的文件就登记进本守卫的 COMPANIONS。来由见 vault 06-BUG/BUG-041。\n{}",
        violations.join("\n")
    );
    // 现有 12 处加锁；数到太少说明记号规则失灵，守卫在空转。
    assert!(total >= 10, "只数到 {total} 处文件锁加锁，守卫规则可能失效");
    for (locking, guard) in COMPANIONS {
        let locking_count = count(&read(&root, locking));
        assert!(
            locking_count.acquired > 0,
            "{locking} 已不再加锁，删掉这条配对"
        );
        let guard_count = count(&read(&root, guard));
        assert!(
            guard_count.released_on_drop > 0,
            "{guard} 里没有带解锁的 Drop"
        );
    }
}

fn locks(acquired: usize, released_on_drop: usize) -> Locks {
    Locks {
        acquired,
        released_on_drop,
    }
}

#[test]
fn lock_without_an_unlocking_drop_is_counted_unreleased() {
    let source = "fn acquire(fd: &OwnedFd) { flock(fd, FlockOperation::LockExclusive)?; }";
    assert_eq!(count(source), locks(1, 0));
    // 只关 FD 的 Drop 不算解锁。
    let closing = [
        source,
        "impl Drop for Lease { fn drop(&mut self) { drop(self.fd.take()); } }",
    ]
    .concat();
    assert_eq!(count(&closing), locks(1, 0));
}

#[test]
fn a_second_lock_needs_its_own_unlocking_drop() {
    // 修复前的出站门铃存储：替换期间的锁有 Serial 守卫放，打开时独占的那把没有。
    let source = [
        "fn open() { flock(&handle, FlockOperation::NonBlockingLockExclusive)?; }",
        "fn serial() { flock(&self.root, FlockOperation::LockExclusive)?; }",
        "impl Drop for Serial { fn drop(&mut self) { flock(self.0, FlockOperation::Unlock); } }",
    ]
    .concat();
    assert_eq!(count(&source), locks(2, 1));
}

#[test]
fn unlocking_drops_pair_with_rustix_and_std_locks() {
    let rustix = [
        "fn acquire() { flock(&lease, FlockOperation::NonBlockingLockExclusive)?; }",
        "impl Drop for Lease { fn drop(&mut self) { flock(&self.0, FlockOperation::Unlock); } }",
    ]
    .concat();
    assert_eq!(count(&rustix), locks(1, 1));
    let std = [
        "fn acquire(file: File) { file.try_lock().map_err(lock_error)?; }",
        "impl<T> std::ops::Drop for Held<T> { fn drop(&mut self) { self.0.unlock(); } }",
    ]
    .concat();
    assert_eq!(count(&std), locks(1, 1));
}

#[test]
fn comments_strings_and_mutexes_are_not_file_locks() {
    let source = [
        "// flock(fd, FlockOperation::LockExclusive)\n",
        "const NOTE: &str = \"flock(fd, FlockOperation::LockExclusive)\";",
        "fn state(&self) { let _a = self.state.lock(); let _b = self.lease.try_lock(); }",
    ]
    .concat();
    assert_eq!(count(&source), Locks::default());
}
