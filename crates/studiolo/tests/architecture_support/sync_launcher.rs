//! 可信 launcher（T-132 P1，ADR-025 D2）的 IO 豁免只登记六个固定文件（按 B 轮 W2 回执的精确记号清单）：
//! `config.rs`（固定配置的两处读法）、`serve.rs`（socket 与目录）、`payload.rs`（垫片：stat、stdio、exec）、
//! `unit.rs`（systemd-run／systemctl 子进程）、`unit/reads.rs`（cgroupfs／proc／session 文件只读）、
//! `unit/session.rs`（session 目录与控制 FIFO）。`facts*`／`handler*`／`session.rs`／`protocol.rs` 保持纯逻辑。
//! 精确调用清单多一个字符都红。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-sync-launcher" {
        return source.to_owned();
    }
    if path.ends_with("crates/sync-launcher/src/config.rs") {
        // `LauncherConfig::load`：symlink_metadata 核普通文件／root／0600／单链接／≤16 KiB，再 read。
        return super::authority_tokens::checked_closed(
            source,
            &["CONFIG_PATH", "CONFIG_MAX_BYTES", "metadata.nlink()==1"],
            &[
                ("usestd::os::unix::fs::MetadataExt;", 1),
                ("std::fs::symlink_metadata(CONFIG_PATH)", 1),
                ("std::fs::read(CONFIG_PATH)", 1),
            ],
        );
    }
    if path.ends_with("crates/sync-launcher/src/serve.rs") {
        // socket 目录与 sessions 目录各经 ensure_dir 一次调用；chmod 一函数供三处；对端凭据必须实测。
        return super::authority_tokens::checked_closed(
            source,
            &[
                "config::SOCKET_DIR",
                "config::SESSIONS_DIR",
                "protocol::LAUNCHER_SOCKET",
                "peer_acceptable(&peer,config.owner_uid,config.owner_gid)",
                "rustix::net::sockopt::socket_peercred(stream)",
            ],
            &[
                ("userustix::fs::Mode;", 1),
                ("UnixListener::bind(&layout.socket)", 1),
                ("std::fs::remove_file(&layout.socket)", 1),
                ("std::fs::create_dir(path)", 1),
                (
                    "rustix::fs::chown(path,None,Some(rustix::process::Gid::from_raw(gid)))",
                    1,
                ),
                ("rustix::fs::chmod(path,mode)", 1),
                ("stream.write_all(&frame)", 1),
            ],
        );
    }
    if path.ends_with("crates/sync-launcher/src/payload.rs") {
        // 垫片：stdin 控制 FIFO（事实帧→放行字节）、stdout 头部写回报告帧、env_clear 后 exec 封闭 argv。
        return super::authority_tokens::checked_closed(
            source,
            &[
                ".env_clear()",
                "service::environment_acceptable(&ENVIRONMENT)",
                ".stdin(Stdio::null())",
                "command.exec()",
            ],
            &[
                ("usestd::os::unix::fs::MetadataExt;", 1),
                ("Command::new(&argv[0])", 1),
                ("stdout.write_all(&frame)", 1),
            ],
        );
    }
    if path.ends_with("crates/sync-launcher/src/unit.rs") {
        // run_transient 一次 systemd-run；全部 systemctl 调用走同一个 fn。本文件没有任何 fs::。
        return super::authority_tokens::checked_closed(
            source,
            &[
                "config::SYSTEMD_RUN",
                "config::SYSTEMCTL",
                "\"--property=\"",
                "command.env_clear();",
                ".stdin(Stdio::null())",
            ],
            &[
                ("Command::new(config::SYSTEMD_RUN)", 1),
                ("Command::new(config::SYSTEMCTL)", 1),
            ],
        );
    }
    if path.ends_with("crates/sync-launcher/src/unit/reads.rs") {
        // 只读：cgroup 事件与限额、/proc/<pid>/exe、session 输出文件的大小与正文。
        return super::authority_tokens::checked_closed(
            source,
            &["\"cgroup.events\"", "facts::CGROUP_ROOT", "\"/proc/{pid}/exe\""],
            &[
                ("usestd::os::unix::fs::MetadataExt;", 1),
                ("std::fs::read_link(format!(\"/proc/{pid}/exe\"))", 1),
                ("std::fs::metadata(path)", 1),
                ("std::fs::read_to_string(path)", 1),
                ("std::fs::read(path)", 1),
            ],
        );
    }
    if path.ends_with("crates/sync-launcher/src/unit/session.rs") {
        // session 目录 0700、控制 FIFO 0600、退休时整目录删除；放行字节经同一 FIFO 写入。
        return super::authority_tokens::checked_closed(
            source,
            &[
                "\"release.fifo\"",
                "\"stdout\"",
                "\"stderr\"",
                ".read(true).write(true)",
            ],
            &[
                ("userustix::fs::{CWD,Mode};", 1),
                ("usestd::fs::File;", 1),
                ("std::fs::create_dir(&path)", 1),
                (
                    "rustix::fs::mkfifoat(CWD,&fifo,Mode::from_raw_mode(0o600))",
                    1,
                ),
                (
                    "std::fs::OpenOptions::new().read(true).write(true).open(&fifo)",
                    1,
                ),
                ("std::fs::remove_dir_all(&self.path)", 1),
                ("self.0.write_all(bytes)", 1),
            ],
        );
    }
    source.to_owned()
}

#[cfg(test)]
mod tests {
    use crate::support::{AuthorityViolation, source_violations, workspace_root};
    use std::{fs, path::Path};

    const CRATE: &str = "studiolo-sync-launcher";
    const FILES: [&str; 6] = [
        "crates/sync-launcher/src/config.rs",
        "crates/sync-launcher/src/serve.rs",
        "crates/sync-launcher/src/payload.rs",
        "crates/sync-launcher/src/unit.rs",
        "crates/sync-launcher/src/unit/reads.rs",
        "crates/sync-launcher/src/unit/session.rs",
    ];

    fn violates(path: &str, source: &str) -> bool {
        source_violations(CRATE, Path::new(path), source)
            .contains(&AuthorityViolation::GenericWrite)
    }

    /// 六个固定文件按登记形态各写一份夹具：正好一次不红，多一处、少一个锚点或换文件名都红。
    fn fixtures() -> [(&'static str, String); 6] {
        let config = [
            "use std::os::unix::fs::MetadataExt;",
            "let metadata = std::fs::symlink_metadata(CONFIG_PATH).map_err(|_| ConfigFault::Unsafe)?;",
            "let unique = metadata.nlink() == 1;",
            "if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o777 != 0o600 || !unique {",
            "    return Err(ConfigFault::Unsafe); }",
            "if metadata.len() > CONFIG_MAX_BYTES as u64 { return Err(ConfigFault::Unsafe); }",
            "let bytes = std::fs::read(CONFIG_PATH).map_err(|_| ConfigFault::Unsafe)?;",
            "Self::parse(&bytes)",
        ];
        let serve = [
            "use rustix::fs::Mode;",
            "let layout = Layout { socket_dir: Path::new(config::SOCKET_DIR), sessions: Path::new(config::SESSIONS_DIR) };",
            "let socket = layout.socket_dir.join(protocol::LAUNCHER_SOCKET);",
            "fn ensure_dir(path: &Path, mode: Mode, gid: u32) -> io::Result<()> {",
            "    std::fs::create_dir(path)?;",
            "    rustix::fs::chown(path, None, Some(rustix::process::Gid::from_raw(gid)))?;",
            "    rustix::fs::chmod(path, mode)?; Ok(()) }",
            "let _ = std::fs::remove_file(&layout.socket);",
            "let listener = UnixListener::bind(&layout.socket)?;",
            "let peer = rustix::net::sockopt::socket_peercred(stream)?;",
            "peer_acceptable(&peer, config.owner_uid, config.owner_gid)?;",
            "stream.write_all(&frame)?;",
        ];
        let payload = [
            "use std::os::unix::fs::MetadataExt;",
            "assert!(service::environment_acceptable(&ENVIRONMENT));",
            "stdout.write_all(&frame)?;",
            "let mut command = Command::new(&argv[0]);",
            "command.env_clear().args(&argv[1..]).stdin(Stdio::null());",
            "let error = command.exec();",
        ];
        let unit = [
            "let mut command = Command::new(config::SYSTEMD_RUN);",
            "command.env_clear();",
            "command.arg(\"--property=\").stdin(Stdio::null());",
            "let show = Command::new(config::SYSTEMCTL).arg(\"show\").output()?;",
        ];
        let reads = [
            "use std::os::unix::fs::MetadataExt;",
            "let root = facts::CGROUP_ROOT;",
            "let events = std::fs::read_to_string(path)?; let _ = \"cgroup.events\";",
            "let exe = std::fs::read_link(format!(\"/proc/{pid}/exe\"))?;",
            "let size = std::fs::metadata(path)?.len();",
            "let bytes = std::fs::read(path)?;",
        ];
        let session = [
            "use rustix::fs::{CWD, Mode};",
            "use std::fs::File;",
            "std::fs::create_dir(&path)?;",
            "let fifo = path.join(\"release.fifo\"); let out = path.join(\"stdout\"); let err = path.join(\"stderr\");",
            "rustix::fs::mkfifoat(CWD, &fifo, Mode::from_raw_mode(0o600))?;",
            "let gate = std::fs::OpenOptions::new().read(true).write(true).open(&fifo)?;",
            "self.0.write_all(bytes)?;",
            "std::fs::remove_dir_all(&self.path)?;",
        ];
        [
            (FILES[0], config.join("\n")),
            (FILES[1], serve.join("\n")),
            (FILES[2], payload.join("\n")),
            (FILES[3], unit.join("\n")),
            (FILES[4], reads.join("\n")),
            (FILES[5], session.join("\n")),
        ]
    }

    #[test]
    fn each_launcher_io_file_is_exempt_only_in_its_registered_shape() {
        for (path, fixture) in fixtures() {
            assert!(!violates(path, &fixture), "{path} 的登记形态本身不该红");
            for extra in [
                "std::fs::write(other, body)?;",
                "File::create(other)?;",
                "file.write_all(body)?;",
                "let listener = UnixListener::bind(other)?;",
                "Command::new(program);",
            ] {
                assert!(
                    violates(path, &format!("{fixture}\n{extra}")),
                    "{path}: {extra}"
                );
            }
            // 同一份代码放到未登记的文件名下：一律红。
            assert!(
                violates("crates/sync-launcher/src/session.rs", &fixture),
                "{path}"
            );
        }
    }

    #[test]
    fn one_call_too_many_or_one_anchor_too_few_voids_the_launcher_exemption() {
        let fixtures = fixtures();
        for (index, duplicated, anchor) in [
            (0, "let again = std::fs::read(CONFIG_PATH)?;", "CONFIG_MAX_BYTES"),
            (1, "std::fs::create_dir(path)?;", "protocol::LAUNCHER_SOCKET"),
            (2, "let shell = Command::new(&argv[0]);", "command.exec()"),
            (3, "let extra = Command::new(config::SYSTEMCTL);", "config::SYSTEMD_RUN"),
            (4, "let more = std::fs::read(path)?;", "facts::CGROUP_ROOT"),
            (5, "std::fs::create_dir(&path)?;", "\"release.fifo\""),
        ] {
            let (path, fixture) = &fixtures[index];
            assert!(
                violates(path, &format!("{fixture}\n{duplicated}")),
                "{path}: {duplicated}"
            );
            assert!(fixture.contains(anchor), "{path}: {anchor}");
            assert!(
                violates(path, &fixture.replace(anchor, "OTHER")),
                "{path}: {anchor}"
            );
        }
        // 当前工作树里真实存在的六个 IO 文件与两个纯逻辑文件都必须过守卫。
        let root = workspace_root();
        for path in FILES
            .iter()
            .copied()
            .chain(["crates/sync-launcher/src/main.rs", "crates/sync-launcher/src/handler.rs"])
        {
            let source = fs::read_to_string(root.join(path)).unwrap();
            assert!(
                source_violations(CRATE, Path::new(path), &source).is_empty(),
                "{path}"
            );
        }
    }
}
