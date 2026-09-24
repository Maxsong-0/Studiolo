//! 可信 launcher（T-132 P1，ADR-025 D2）的 IO 豁免只登记四个固定文件：
//! `config.rs`（读固定配置的两处 IO）、`serve.rs`（socket、目录、FIFO、mountinfo／boot_id 读取）、
//! `payload.rs`（垫片：stat、FIFO、exec）、`unit.rs`（systemd-run／systemctl 与 cgroup 属性读回）。
//! 其余文件保持纯逻辑；精确调用清单按 W2 任务书第 2 节的固定形态登记，多一个字符都红。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-sync-launcher" {
        return source.to_owned();
    }
    if path.ends_with("crates/sync-launcher/src/config.rs") {
        // `LauncherConfig::load`：symlink_metadata 核普通文件／root／0600／单链接／≤16 KiB，再 read。
        return super::authority_tokens::checked_closed(
            source,
            &["CONFIG_PATH", "CONFIG_MAX_BYTES"],
            &[
                ("std::fs::symlink_metadata(CONFIG_PATH)", 1),
                ("std::fs::read(CONFIG_PATH)", 1),
                ("std::os::unix::fs::MetadataExt", 1),
            ],
        );
    }
    if path.ends_with("crates/sync-launcher/src/serve.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "config::SOCKET_DIR",
                "config::SESSIONS_DIR",
                "config::SYSTEMD_RUN",
                "peer_acceptable(&peer,config.owner_uid,config.owner_gid)",
            ],
            &[
                ("UnixListener::bind(", 1),
                ("std::fs::create_dir(", 2),
                ("std::fs::read_to_string(", 3),
                ("std::fs::symlink_metadata(", 1),
                ("rustix::fs::mkfifo(", 2),
                ("rustix::fs::chmod(", 2),
                ("rustix::fs::chown(", 1),
            ],
        );
    }
    if path.ends_with("crates/sync-launcher/src/payload.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &["LISTEN_FDNAMES", ".env_clear()", "environment_acceptable("],
            &[
                ("Command::new(&argv[0])", 1),
                ("std::os::unix::fs::MetadataExt", 1),
            ],
        );
    }
    if path.ends_with("crates/sync-launcher/src/unit.rs") {
        // run_transient 一次 systemd-run；show／stop 各一次 systemctl；events＋四项限额各读一次。
        return super::authority_tokens::checked_closed(
            source,
            &["\"--property=\"", "\"cgroup.events\""],
            &[
                ("Command::new(config::SYSTEMD_RUN)", 1),
                ("Command::new(config::SYSTEMCTL)", 2),
                ("std::fs::read_to_string(", 5),
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

    fn violates(path: &str, source: &str) -> bool {
        source_violations(CRATE, Path::new(path), source)
            .contains(&AuthorityViolation::GenericWrite)
    }

    /// 四个固定文件按登记形态各写一份夹具：正好一次不红，多一处、少一个锚点或换文件名都红。
    fn fixtures() -> [(&'static str, String); 4] {
        let config = [
            "use std::os::unix::fs::MetadataExt;",
            "let meta = std::fs::symlink_metadata(CONFIG_PATH).map_err(|_| ConfigFault::Unsafe)?;",
            "if !meta.is_file() || meta.uid() != 0 || meta.mode() & 0o777 != 0o600 || meta.nlink() != 1 {",
            "    return Err(ConfigFault::Unsafe); }",
            "if meta.len() > CONFIG_MAX_BYTES as u64 { return Err(ConfigFault::Unsafe); }",
            "let bytes = std::fs::read(CONFIG_PATH).map_err(|_| ConfigFault::Unsafe)?;",
            "Self::parse(&bytes)",
        ];
        let serve = [
            "let socket = Path::new(config::SOCKET_DIR).join(\"launcher.sock\");",
            "let sessions = Path::new(config::SESSIONS_DIR);",
            "let runner = config::SYSTEMD_RUN;",
            "std::fs::create_dir(config::SOCKET_DIR)?;",
            "std::fs::create_dir(config::SESSIONS_DIR)?;",
            "rustix::fs::chown(config::SOCKET_DIR, Some(root), Some(group))?;",
            "rustix::fs::chmod(config::SOCKET_DIR, Mode::from_raw_mode(0o750))?;",
            "let listener = UnixListener::bind(&socket)?;",
            "rustix::fs::chmod(&socket, Mode::from_raw_mode(0o660))?;",
            "let peer = stream.peer_cred()?;",
            "peer_acceptable(&peer, config.owner_uid, config.owner_gid)?;",
            "let mountinfo = std::fs::read_to_string(\"/proc/self/mountinfo\")?;",
            "let backing = std::fs::read_to_string(backing_file)?;",
            "let boot = std::fs::read_to_string(\"/proc/sys/kernel/random/boot_id\")?;",
            "let meta = std::fs::symlink_metadata(&session_dir)?;",
            "rustix::fs::mkfifo(session_dir.join(\"release.fifo\"), Mode::from_raw_mode(0o600))?;",
            "rustix::fs::mkfifo(session_dir.join(\"report.fifo\"), Mode::from_raw_mode(0o600))?;",
        ];
        let payload = [
            "use std::os::unix::fs::MetadataExt;",
            "let names = std::env::var(\"LISTEN_FDNAMES\")?;",
            "assert!(environment_acceptable(&[\"XDG_CONFIG_HOME\", \"LANG\", \"TZ\"]));",
            "let mut command = Command::new(&argv[0]);",
            "command.env_clear().env(\"LANG\", \"C.UTF-8\").args(&argv[1..]);",
        ];
        let unit = [
            "const EVENTS: &str = \"cgroup.events\";",
            "let mut command = Command::new(config::SYSTEMD_RUN);",
            "command.env_clear().arg(\"--property=\").arg(\"KillMode=control-group\");",
            "let show = Command::new(config::SYSTEMCTL).arg(\"show\").output()?;",
            "let stop = Command::new(config::SYSTEMCTL).arg(\"stop\").output()?;",
            "let events = std::fs::read_to_string(format!(\"/sys/fs/cgroup{group}/{EVENTS}\"))?;",
            "let memory = std::fs::read_to_string(format!(\"/sys/fs/cgroup{group}/memory.max\"))?;",
            "let swap = std::fs::read_to_string(format!(\"/sys/fs/cgroup{group}/memory.swap.max\"))?;",
            "let pids = std::fs::read_to_string(format!(\"/sys/fs/cgroup{group}/pids.max\"))?;",
            "let cpu = std::fs::read_to_string(format!(\"/sys/fs/cgroup{group}/cpu.max\"))?;",
        ];
        [
            ("crates/sync-launcher/src/config.rs", config.join("\n")),
            ("crates/sync-launcher/src/serve.rs", serve.join("\n")),
            ("crates/sync-launcher/src/payload.rs", payload.join("\n")),
            ("crates/sync-launcher/src/unit.rs", unit.join("\n")),
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
        for (path, fixture, duplicated, anchor) in [
            (
                "crates/sync-launcher/src/config.rs",
                fixtures()[0].1.clone(),
                "let again = std::fs::read(CONFIG_PATH)?;",
                "CONFIG_MAX_BYTES",
            ),
            (
                "crates/sync-launcher/src/serve.rs",
                fixtures()[1].1.clone(),
                "std::fs::create_dir(other)?;",
                "config::SYSTEMD_RUN",
            ),
            (
                "crates/sync-launcher/src/payload.rs",
                fixtures()[2].1.clone(),
                "let shell = Command::new(&argv[0]);",
                "LISTEN_FDNAMES",
            ),
            (
                "crates/sync-launcher/src/unit.rs",
                fixtures()[3].1.clone(),
                "let extra = std::fs::read_to_string(other)?;",
                "\"cgroup.events\"",
            ),
        ] {
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
        // 当前工作树里真实存在的文件也必须过守卫。
        let root = workspace_root();
        for path in [
            "crates/sync-launcher/src/config.rs",
            "crates/sync-launcher/src/main.rs",
        ] {
            let source = fs::read_to_string(root.join(path)).unwrap();
            assert!(
                source_violations(CRATE, Path::new(path), &source).is_empty(),
                "{path}"
            );
        }
    }
}
