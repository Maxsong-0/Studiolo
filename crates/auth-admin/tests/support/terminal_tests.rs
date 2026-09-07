//! 纯输入契约与真实 PTY 子进程；生产源码没有测试进程或虚构输入入口。

use super::*;
use std::{path::PathBuf, process::Command, time::Duration};

#[test]
fn input_is_six_ascii_digits_and_backspace_erases_owned_buffer() {
    let mut input = Input::default();
    for byte in b"48271" {
        assert!(input.push(*byte).unwrap().is_none());
    }
    assert!(input.push(127).unwrap().is_none());
    assert_eq!(input.length, 4);
    assert_eq!(input.digits[4], 0);
    for byte in b"19" {
        assert!(input.push(*byte).unwrap().is_none());
    }
    assert!(input.push(b'\n').unwrap().is_some());
}

#[test]
fn malformed_cancelled_and_eof_input_never_produce_a_code() {
    for bytes in [
        b"\n".as_slice(),
        b"48271\n",
        b"4827190",
        "４８２７１９".as_bytes(),
        b"48271x",
        b" 482719",
    ] {
        let mut input = Input::default();
        let failure = bytes.iter().find_map(|byte| input.push(*byte).err());
        assert_eq!(failure, Some(TerminalError::InvalidCode));
    }
    for (byte, error) in [(3, TerminalError::Cancelled), (4, TerminalError::Eof)] {
        assert_eq!(Input::default().push(byte).err(), Some(error));
    }
}

#[test]
fn real_controlling_tty_contract() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/terminal_pty.py");
    let output = Command::new("python3")
        .arg(script)
        .arg(std::env::current_exe().unwrap())
        .output()
        .expect("PTY 契约必须安装 Python3，不能静默跳过");
    assert!(
        output.status.success(),
        "PTY 契约失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn terminal_probe() {
    let Ok(scenario) = std::env::var("STUDIOLO_TERMINAL_PROBE") else {
        return;
    };
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok();
    let before = tty.as_ref().and_then(|tty| termios::tcgetattr(tty).ok());
    #[cfg(target_os = "macos")]
    if let Some(original) = &before {
        canonical_transition_rejects_unrelated_changes(original);
    }
    let result = if scenario == "preflight" {
        preflight()
    } else {
        read_code().map(|_| ())
    };
    let after = tty.as_ref().and_then(|tty| termios::tcgetattr(tty).ok());
    let restored = match (before, after) {
        (Some(before), Some(after)) => {
            after.local_modes == before.local_modes
                && after.input_modes == before.input_modes
                && after.output_modes == before.output_modes
                && after.control_modes == before.control_modes
                && [
                    termios::SpecialCodeIndex::VMIN,
                    termios::SpecialCodeIndex::VTIME,
                ]
                .into_iter()
                .all(|code| after.special_codes[code] == before.special_codes[code])
        }
        (None, None) => scenario == "preflight",
        _ => false,
    };
    println!("PROBE_RESULT={result:?};RESTORED={restored}");
    if scenario == "after_signal" && result.is_ok() {
        println!("POST_READ_READY");
        // Python 发真实 SIGINT/SIGHUP；若默认行为没有恢复，本测试明确失败而非挂起。
        std::thread::sleep(Duration::from_secs(10));
        panic!("输入结束后默认信号行为未恢复");
    }
}

#[cfg(target_os = "macos")]
fn canonical_transition_rejects_unrelated_changes(original: &termios::Termios) {
    use termios::{LocalModes, SpecialCodeIndex as Code};
    let mut expected = original.clone();
    expected.local_modes.insert(LocalModes::ICANON);
    expected.local_modes.remove(LocalModes::PENDIN);
    let mut actual = expected.clone();
    actual.local_modes.insert(LocalModes::PENDIN);
    assert!(mode::canonical_transition(&actual, &expected));
    actual.local_modes.toggle(LocalModes::ECHO);
    assert!(!mode::canonical_transition(&actual, &expected));
    actual.local_modes.toggle(LocalModes::ECHO);
    actual.special_codes[Code::VMIN] ^= 1;
    assert!(!mode::canonical_transition(&actual, &expected));
    assert!(!mode::canonical_transition(&expected, &expected));
}
