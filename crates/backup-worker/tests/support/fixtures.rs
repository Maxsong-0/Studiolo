//! 固定包/真实进程边界测试；所有环境内容和仓库口令都是虚构值。
use crate::{ProcessError, ResticCredentials, ResticPackage, process};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn hash(path: &Path) -> String {
    Sha256::digest(fs::read(path).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn package() -> ResticPackage {
    let path = std::env::current_exe().unwrap();
    ResticPackage::fixture(&path, &hash(&path), deadline()).unwrap()
}
fn args(name: &str) -> Vec<String> {
    vec![
        "--exact".into(),
        format!("fixtures::{name}"),
        "--ignored".into(),
        "--nocapture".into(),
    ]
}

#[test]
fn original_executable_is_hashed_and_mode_or_content_changes_refuse_execution() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("executable");
    fs::copy(std::env::current_exe().unwrap(), &path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
    let digest = hash(&path);
    assert!(matches!(
        ResticPackage::fixture(&path, &"0".repeat(64), deadline()),
        Err(ProcessError::Package)
    ));
    let original = ResticPackage::fixture(&path, &digest, deadline()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(original.executable(), Err(ProcessError::Package));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
    let original = ResticPackage::fixture(&path, &digest, deadline()).unwrap();
    fs::rename(&path, temp.path().join("old")).unwrap();
    fs::write(&path, b"not the verified executable").unwrap();
    // rename 改变原 inode 的 ctime，或继续使用原 FD；绝不能执行替换文件。
    if original.executable().is_ok() {
        let output = process::run(
            &original,
            &args("child_environment"),
            None,
            temp.path(),
            deadline(),
        )
        .unwrap();
        assert!(output.success);
    }
}

#[test]
fn child_has_only_explicit_environment_and_bounded_output() {
    let temp = tempfile::tempdir().unwrap();
    let output = process::run(
        &package(),
        &args("child_environment"),
        None,
        temp.path(),
        deadline(),
    )
    .unwrap();
    assert!(output.success);
    assert!(String::from_utf8_lossy(&output.stdout).contains("fixed-environment-ok"));
    for child in ["child_stdout_overflow", "child_stderr_overflow"] {
        assert!(matches!(
            process::run(&package(), &args(child), None, temp.path(), deadline()),
            Err(ProcessError::OutputLimit)
        ));
    }
}

#[test]
fn timeout_and_exit_with_inherited_pipe_kill_group_before_reaping_leader() {
    let temp = tempfile::tempdir().unwrap();
    let package = package();
    let started = Instant::now();
    assert!(matches!(
        process::run(
            &package,
            &args("child_hangs"),
            None,
            temp.path(),
            Instant::now() + Duration::from_millis(150)
        ),
        Err(ProcessError::Deadline)
    ));
    assert!(started.elapsed() < Duration::from_secs(4));
    let expires = Instant::now() + Duration::from_millis(100);
    assert!(matches!(
        process::run_scoped(
            &package,
            &args("child_hangs"),
            None,
            temp.path(),
            deadline(),
            &|| {
                if Instant::now() >= expires {
                    Err(ProcessError::Deadline)
                } else {
                    Ok(())
                }
            }
        ),
        Err(ProcessError::Deadline)
    ));
    assert!(started.elapsed() < Duration::from_secs(4));
    let output = process::run(
        &package,
        &args("child_leaves_descendant"),
        None,
        temp.path(),
        deadline(),
    )
    .unwrap();
    assert!(output.success);
    let text = String::from_utf8_lossy(&output.stdout);
    let pid = text
        .lines()
        .find_map(|line| line.strip_prefix("descendant="))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    // 被 init 接管的孙进程可能短暂为 zombie；它已不能继续写标记文件。
    std::thread::sleep(Duration::from_millis(700));
    assert!(!temp.path().join("descendant-survived").exists());
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => assert!(stat.rsplit_once(") ").unwrap().1.starts_with('Z')),
        Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::NotFound),
    }
}

#[test]
fn repository_checks_yield_on_request_and_bound_stderr_at_four_mebibytes() {
    let temp = tempfile::tempdir().unwrap();
    let package = package();
    let credentials = ResticCredentials::fixture(&temp.path().join("password"));
    let check = |name: &str, interrupt: &dyn Fn() -> crate::Result<()>| {
        process::run_check(
            &package,
            &args(name),
            &credentials,
            temp.path(),
            deadline(),
            &|| Ok(()),
            interrupt,
        )
    };
    // 让路：回调一报错就结束整组进程，原样返回让路错误，不等到截止时间。
    let started = Instant::now();
    assert!(matches!(
        check("child_hangs", &|| Err(ProcessError::Interrupted)),
        Err(ProcessError::Interrupted)
    ));
    assert!(started.elapsed() < Duration::from_secs(4));
    // 普通调用 64 KiB 就越界的 stderr，抽查容得下；超过 4 MiB 照样越界。
    assert!(check("child_stderr_overflow", &|| Ok(())).unwrap().success);
    assert!(matches!(
        check("child_stderr_flood", &|| Ok(())),
        Err(ProcessError::OutputLimit)
    ));
}

#[test]
#[ignore = "由父测试作为真实独立子进程调用"]
fn child_environment() {
    assert!(std::env::var_os("STUDIOLO_INHERITED_POISON").is_none());
    assert!(std::env::var_os("HOME").is_none());
    assert!(std::env::var_os("PATH").is_none());
    assert!(std::env::var_os("RESTIC_PASSWORD_COMMAND").is_none());
    assert_eq!(std::env::vars_os().count(), 1);
    println!("fixed-environment-ok");
}
#[test]
#[ignore = "由父测试作为真实独立子进程调用"]
fn child_stdout_overflow() {
    std::io::stdout()
        .write_all(&vec![b'x'; 9 * 1024 * 1024])
        .unwrap();
}
#[test]
#[ignore = "由父测试作为真实独立子进程调用"]
fn child_stderr_overflow() {
    std::io::stderr()
        .write_all(&vec![b'x'; 128 * 1024])
        .unwrap();
}
#[test]
#[ignore = "由父测试作为真实独立子进程调用"]
fn child_stderr_flood() {
    std::io::stderr()
        .write_all(&vec![b'x'; 5 * 1024 * 1024])
        .unwrap();
}
#[test]
#[ignore = "由父测试作为真实独立子进程调用"]
fn child_hangs() {
    std::thread::sleep(Duration::from_secs(30));
}
#[test]
#[ignore = "由父测试作为真实独立子进程调用"]
fn child_leaves_descendant() {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(args("child_descendant"))
        .spawn()
        .unwrap();
    println!("descendant={}", child.id());
    std::io::stdout().flush().unwrap();
    std::mem::forget(child);
}
#[test]
#[ignore = "由父测试作为真实独立子进程调用"]
fn child_descendant() {
    std::thread::sleep(Duration::from_millis(500));
    fs::write(
        Path::new(&std::env::var_os("TMPDIR").unwrap()).join("descendant-survived"),
        b"not killed",
    )
    .unwrap();
    std::thread::sleep(Duration::from_secs(30));
}

mod restic_cli;

#[test]
fn bounded_stdin_delivers_eof_without_blocking_output_or_bypassing_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let password = temp.path().join("password");
    fs::write(&password, "fictional-only\n").unwrap();
    let credentials = ResticCredentials::fixture(&password);
    let package = package();
    let bytes = vec![0x5a; 64 * 1024];
    let output = process::run_input(
        &package,
        &args("child_stdin"),
        &credentials,
        temp.path(),
        deadline(),
        &|| Ok(()),
        &bytes,
    )
    .unwrap();
    assert!(output.success);
    assert!(String::from_utf8_lossy(&output.stdout).contains("input-ok"));
    assert!(matches!(
        process::run_input(
            &package,
            &args("child_stdin"),
            &credentials,
            temp.path(),
            deadline(),
            &|| Ok(()),
            &vec![0; 65537]
        ),
        Err(ProcessError::Configuration)
    ));
    assert!(matches!(
        process::run_input(
            &package,
            &args("child_hangs"),
            &credentials,
            temp.path(),
            Instant::now() + Duration::from_millis(150),
            &|| Ok(()),
            &bytes
        ),
        Err(ProcessError::Deadline)
    ));
}
#[test]
#[ignore = "由父测试交付有界虚构 stdin"]
fn child_stdin() {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(65537)
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, vec![0x5a; 64 * 1024]);
    println!("input-ok");
}
