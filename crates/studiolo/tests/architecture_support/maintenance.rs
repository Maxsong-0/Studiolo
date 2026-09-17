//! 离线事务维护只得到固定控制 TTY；不能取得任意文件或子进程权限。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-local-server"
        || !path.ends_with("crates/local-server/src/maintenance/terminal.rs")
    {
        return source.to_owned();
    }
    let mut remainder = source
        .split_whitespace()
        .collect::<String>()
        .replace(",)", ")");
    let expected = "fs::open(\"/dev/tty\",fs::OFlags::RDWR|fs::OFlags::CLOEXEC|fs::OFlags::NOCTTY|fs::OFlags::NONBLOCK,fs::Mode::empty())";
    if remainder.matches("fs::open(").count() != 1 || !remainder.contains(expected) {
        return remainder;
    }
    for allowed in ["fs::File,", expected, "fs::fstat(&input)", "fs::fstat(fd)"] {
        remainder = remainder.replace(allowed, "");
    }
    remainder
}
