//! `studiolo-restore` 只认本机终端（T-212 M2）：真跑一遍编出来的程序，标准错误不是终端时什么都不读、不写，
//! 以固定原因、退出码 1 拒绝；参数不对在那之前就以退出码 2 报用法。
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

const CHECKS: [&str; 8] = [
    "sync-adapter-disabled",
    "execution-adapter-disabled",
    "content-verified",
    "version-verified",
    "permissions-verified",
    "hashes-verified",
    "audit-continuity-verified",
    "space-verified",
];

fn restore(args: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_studiolo-restore"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("恢复命令行必须能启动")
}

fn owned(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn run_args(root: &Path) -> Vec<String> {
    let config = root.join("owner.json").display().to_string();
    let copy = root.join("copy").display().to_string();
    let new_root = root.join("new-root").display().to_string();
    let seal = "ab".repeat(32);
    let mut args = owned(&[
        "run",
        "--config",
        &config,
        "--checkpoint-copy",
        &copy,
        "--checkpoint",
        "cp-0001",
        "--seal-sha256",
        &seal,
        "--new-root",
        &new_root,
        "--restore-id",
        "drill-1",
    ]);
    for name in CHECKS {
        args.extend(owned(&["--confirm", name]));
    }
    args
}

#[test]
fn outside_a_terminal_every_subcommand_is_refused_before_touching_anything() {
    let scratch = tempfile::tempdir().expect("临时目录");
    let new_root = scratch.path().join("new-root");
    fs::create_dir(&new_root).expect("新根");
    let config = scratch.path().join("owner.json").display().to_string();
    let root = new_root.display().to_string();
    let cutover = |step: &str| {
        owned(&[
            "cutover",
            step,
            "--config",
            &config,
            "--new-root",
            &root,
            "--restore-id",
            "drill-1",
        ])
    };
    let mut resumed = run_args(scratch.path());
    resumed.push("--resume".to_owned());
    for args in [
        run_args(scratch.path()),
        resumed,
        cutover("begin"),
        cutover("finish"),
    ] {
        let output = restore(&args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "拒绝时标准输出不能有恢复记录");
        let message = String::from_utf8(output.stderr).expect("中文原因");
        assert!(message.contains("只认本机终端"), "{message}");
        assert!(
            !message.contains("秘密驻留面"),
            "拒绝发生在加固和读任何输入之前"
        );
    }
    assert_eq!(fs::read_dir(&new_root).expect("新根").count(), 0);
    assert_eq!(fs::read_dir(scratch.path()).expect("临时目录").count(), 1);
}

#[test]
fn malformed_arguments_report_usage_before_the_terminal_check() {
    let scratch = tempfile::tempdir().expect("临时目录");
    let mut relative = run_args(scratch.path());
    let index = relative
        .iter()
        .position(|value| value == "--new-root")
        .expect("有新根参数");
    relative[index + 1] = "new-root".to_owned();
    let mut unconfirmed = run_args(scratch.path());
    let index = unconfirmed
        .iter()
        .position(|value| value == "space-verified")
        .expect("有空间检查");
    unconfirmed[index] = "everything-verified".to_owned();
    for args in [
        Vec::new(),
        owned(&["cutover", "abort"]),
        relative,
        unconfirmed,
    ] {
        let output = restore(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
        let message = String::from_utf8(output.stderr).expect("中文原因");
        assert!(message.contains("用法"), "{message}");
    }
}
