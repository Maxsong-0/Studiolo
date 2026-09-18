//! 可信本地工具只获明确只读配置/控制TTY权限，不获通用文件写入或进程执行能力。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if !matches!(
        crate_name,
        "studiolo-auth-admin" | "studiolo-host-config" | "studiolo-agent-server"
    ) {
        return source.to_owned();
    }
    let mut remainder = source
        .split_whitespace()
        .collect::<String>()
        .replace(",)", ")");
    if crate_name == "studiolo-host-config"
        && path.ends_with("crates/host-config/src/config_file.rs")
    {
        let expected = "open(path,OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::NONBLOCK|OFlags::CLOEXEC,Mode::empty())";
        if remainder.matches("open(").count() != 1 || !remainder.contains(expected) {
            return remainder;
        }
        for allowed in [
            "rustix::fs::{open,Mode,OFlags}",
            "rustix::fs::{Mode,OFlags,open}",
            "fs::{self,File,Metadata}",
            "os::unix::fs::MetadataExt",
            "fs::symlink_metadata(",
            expected,
        ] {
            remainder = remainder.replace(allowed, "");
        }
    } else if crate_name == "studiolo-auth-admin"
        && path.ends_with("crates/auth-admin/src/terminal.rs")
    {
        let expected =
            "fs::open(\"/dev/tty\",OFlags::RDWR|OFlags::CLOEXEC|OFlags::NOCTTY,Mode::empty())";
        if remainder.matches("fs::open(").count() != 1 || !remainder.contains(expected) {
            return remainder;
        }
        for allowed in [
            "fs::{self,Mode,OFlags}",
            "fs::File,",
            expected,
            "fs::fstat(&input)",
            "fs::fstat(&error)",
            "(&hidden.file).write_all(\"请输入六位验证码（不回显）：\".as_bytes())",
            "(&hidden.file).write_all(b\"\\r\\n\")",
        ] {
            remainder = remainder.replace(allowed, "");
        }
    } else if crate_name == "studiolo-auth-admin"
        && path.ends_with("crates/auth-admin/src/terminal_mode.rs")
    {
        remainder = remainder.replace("usestd::fs::File;", "");
    } else if crate_name == "studiolo-agent-server"
        && path.ends_with("crates/agent-server/src/admin/terminal.rs")
    {
        // 类型 token 只写固定控制终端；与离线维护 CLI 同一套调用点，不得扩到别的文件或子进程。
        let expected = "fs::open(\"/dev/tty\",fs::OFlags::RDWR|fs::OFlags::CLOEXEC|fs::OFlags::NOCTTY|fs::OFlags::NONBLOCK,fs::Mode::empty())";
        if remainder.matches("fs::open(").count() != 1 || !remainder.contains(expected) {
            return remainder;
        }
        for allowed in ["fs::File,", expected, "fs::fstat(&input)", "fs::fstat(fd)"] {
            remainder = remainder.replace(allowed, "");
        }
    } else if crate_name == "studiolo-agent-server"
        && path.ends_with("crates/agent-server/src/admin/policy.rs")
    {
        // 策略 bootstrap 的唯一文件读取：主人显式指定的正文文件，读一次、有界；不得扩成通用读写。
        let expected = "std::fs::read(file)";
        if remainder.matches("fs::").count() != 1 || !remainder.contains(expected) {
            return remainder;
        }
        remainder = remainder.replace(expected, "");
    }
    remainder
}
