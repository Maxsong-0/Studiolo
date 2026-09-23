//! 可信 launcher（T-132 P1，ADR-025 D2）的 IO 豁免只登记三个固定文件：
//! `serve.rs`（socket、systemd-run／systemctl、FIFO、statfs）、`payload.rs`（垫片：stat、FIFO、exec）、
//! `unit.rs`（systemd 属性读回）。其余文件保持纯逻辑；精确调用清单由 W1 按 W2 的固定形态收口。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-sync-launcher" {
        return source.to_owned();
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
                ("Command::new(config::SYSTEMD_RUN)", 1),
                ("Command::new(config::SYSTEMCTL)", 1),
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
        return super::authority_tokens::checked_closed(
            source,
            &["\"--property=\"", "\"cgroup.events\""],
            &[
                ("Command::new(config::SYSTEMCTL)", 1),
                ("std::fs::read_to_string(", 1),
            ],
        );
    }
    source.to_owned()
}
