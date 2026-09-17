//! 测试使用系统 script 装配真实 PTY，不把 test helper 或短期限编入产品入口。

use std::{
    ffi::OsString,
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub fn script(program: &Path, args: &[OsString]) -> Command {
    assert!(
        Path::new("/usr/bin/script").is_file(),
        "真实 PTY 契约需要系统 /usr/bin/script"
    );
    let mut command = Command::new("/usr/bin/script");
    #[cfg(target_os = "macos")]
    command.arg("-q").arg("/dev/null").arg(program).args(args);
    #[cfg(target_os = "linux")]
    {
        let shell_word = |value: &std::ffi::OsStr| {
            format!("'{}'", value.to_str().unwrap().replace('\'', "'\\''"))
        };
        let mut words = vec![shell_word(program.as_os_str())];
        words.extend(args.iter().map(|value| shell_word(value)));
        command
            .args(["-q", "-e", "-c"])
            .arg(words.join(" "))
            .arg("/dev/null");
    }
    command
}

pub fn output(mut command: Command, input: Option<&[u8]>) -> (std::process::ExitStatus, String) {
    let mut child = OwnedChild(
        command
            .env_remove("STUDIOLO_TEST_MAINTAIN_BIN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    // 等待退出前持续读取两条管道；较长的真实终端审阅不能被测试自身堵住。
    let stdout = drain(child.0.stdout.take().unwrap());
    let stderr = drain(child.0.stderr.take().unwrap());
    if let Some(input) = input {
        child.0.stdin.as_mut().unwrap().write_all(input).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "子进程未在有界期限内退出");
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut bytes = stdout.join().unwrap();
    bytes.extend(stderr.join().unwrap());
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn drain(reader: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let limit = 1024 * 1024;
        let mut bytes = Vec::new();
        reader.take(limit + 1).read_to_end(&mut bytes).unwrap();
        assert!(bytes.len() <= limit as usize, "子进程输出超出测试预算");
        bytes
    })
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
