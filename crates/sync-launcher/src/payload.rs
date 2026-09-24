//! `studiolo-sync-launcher payload`：transient unit 内的垫片。stdin 是 launcher 预写好的控制 FIFO：
//! 先读一帧启动事实（固定 pins、镜像根、配置根、操作），在最终 mount 视图内 stat 四个对象，把
//! `PayloadReport` 一帧写到 stdout（文件头部，launcher 按长度前缀切分），再阻塞读一字节放行；
//! 放行前读到 EOF 就是作废。放行后 `env_clear` 只加三个批准变量，`exec` 封闭 argv；
//! 不读配置文件、不认参数、不搜 PATH、stdin 换成 /dev/null 再 exec。
//!
//! 退出码：3＝事实帧缺失／不合法或 stat 失败，4＝放行前作废，5＝环境或 exec 失败。

use crate::facts::{self, LaunchFacts};
use crate::protocol::{FRAME_MAX_BYTES, PayloadReport, PayloadReportKind, encode_payload_report};
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;
use studiolo_sync_transport::linux::service;

const EXIT_NO_FACTS: u8 = 3;
const EXIT_GATED: u8 = 4;
const EXIT_EXEC: u8 = 5;
/// 只有这三个变量进 payload 的环境；`service::environment_acceptable` 是第二道自证。
const ENVIRONMENT: [&str; 3] = ["XDG_CONFIG_HOME", "LANG", "TZ"];

/// 垫片主流程；只在 exec 成功时不返回。
pub fn run() -> ExitCode {
    let mut stdin = std::io::stdin().lock();
    let Some(facts) = read_facts(&mut stdin) else {
        return fail(EXIT_NO_FACTS, "垫片：控制 FIFO 上没有合法的启动事实帧");
    };
    let Some(report) = report(&facts) else {
        return fail(EXIT_NO_FACTS, "垫片：最终视图内 stat 四个对象失败");
    };
    let Ok(frame) = encode_payload_report(&report) else {
        return fail(EXIT_NO_FACTS, "垫片：报告帧编码失败");
    };
    let written = {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&frame).and_then(|()| stdout.flush())
    };
    if written.is_err() {
        return fail(EXIT_NO_FACTS, "垫片：报告帧写不进 stdout");
    }
    if !released(&mut stdin) {
        return fail(EXIT_GATED, "垫片：放行前控制 FIFO 已关闭，不执行");
    }
    drop(stdin);
    exec(&facts)
}

fn read_facts<R: Read>(input: &mut R) -> Option<LaunchFacts> {
    let mut header = [0u8; 4];
    input.read_exact(&mut header).ok()?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > FRAME_MAX_BYTES {
        return None;
    }
    let mut frame = vec![0u8; 4 + length];
    frame[..4].copy_from_slice(&header);
    input.read_exact(&mut frame[4..]).ok()?;
    facts::decode_launch_facts(&frame).ok()
}

/// 四个对象跟随链接后的 `(dev, ino)`：launcher 在自己的视图里用同一读法核对。
fn report(facts: &LaunchFacts) -> Option<PayloadReport> {
    let identity = |path: &Path| path.metadata().ok().map(|meta| (meta.dev(), meta.ino()));
    Some(PayloadReport {
        kind: PayloadReportKind::PayloadReport,
        mirror_identity: identity(&facts.mirror_root)?,
        config_identity: identity(&facts.payload_config_root)?,
        node_identity: identity(&facts.runtime.node_path)?,
        cli_identity: identity(&facts.runtime.cli_path)?,
        uid: rustix::process::getuid().as_raw(),
        gid: rustix::process::getgid().as_raw(),
    })
}

/// 阻塞等一字节；EOF 即作废。非阻塞或被中断时重试，不把它们当作放行。
fn released<R: Read>(input: &mut R) -> bool {
    let mut byte = [0u8; 1];
    loop {
        match input.read(&mut byte) {
            Ok(1) => return true,
            Ok(_) => return false,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) =>
            {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return false,
        }
    }
}

fn exec(facts: &LaunchFacts) -> ExitCode {
    if !service::environment_acceptable(&ENVIRONMENT) {
        return fail(EXIT_EXEC, "垫片：批准的环境变量名不可接受");
    }
    let Ok(argv) = service::build_argv(&facts.runtime, facts.operation, &facts.mirror_root) else {
        return fail(EXIT_EXEC, "垫片：封闭 argv 构造失败");
    };
    let mut command = Command::new(&argv[0]);
    command
        .env_clear()
        .env(ENVIRONMENT[0], &facts.payload_config_root)
        .env(ENVIRONMENT[1], "C.UTF-8")
        .env(ENVIRONMENT[2], "UTC")
        .args(&argv[1..])
        .stdin(Stdio::null());
    let error = command.exec();
    fail(EXIT_EXEC, &format!("垫片：exec 失败：{error}"))
}

fn fail(code: u8, message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(code)
}
