//! SyncTransportOwner 只写固定的 transport-private 单槽；运行日志正文不落这里。
//! `linux/client.rs`（T-132 P1，ADR-025 D3）是唯一的 launcher IPC 文件：只连固定 socket、
//! 只收发有界帧；不开文件、不起子进程。crate 里别的文件连端点记号都不许出现。

use std::path::Path;

/// 只有 `client.rs` 的精确清单能持有的端点记号；出现在别处或超出清单即视为越权。
const SOCKET_TOKENS: [&str; 3] = ["UnixStream::connect", "UnixListener", "TcpStream"];

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-sync-transport" {
        return source.to_owned();
    }
    if path.ends_with("crates/sync-transport/src/linux/client.rs") {
        return client_remainder(source);
    }
    if !path.ends_with("crates/sync-transport/src/store/publish.rs") {
        return socket_free(source.to_owned());
    }
    let mut rest = source.split_whitespace().collect::<String>();
    // 固定文件名常量必须在场，换掉固定名就拿不到豁免。
    for required in [
        "paths::TRANSPORT_LOCK",
        "paths::STATE",
        "paths::STATE_PUBLISH_TEMP",
    ] {
        if !rest.contains(required) {
            return rest;
        }
    }
    for exact in [
        "fs::{File,TryLockError},",
        "userustix::fs::{AtFlags,unlinkat};",
        "userustix::fs::{Mode,OFlags,openat};",
        "userustix::fs::{Mode,OFlags,openat,renameat};",
        "userustix::fs::{Mode,OFlags,open,openat};",
        "usestd::os::unix::fs::MetadataExt;",
        "lease.0.sync_all()",
        "temp.write_all(payload)",
        "temp.sync_all()",
        "self.root.sync_all()",
        "root.sync_all()",
    ] {
        rest = rest.replace(exact, "");
    }
    rest
}

/// `client.rs` 的精确调用清单（W1 收口）：一次连接、一次写帧、两次定长读（长度头与载荷）、
/// 一次 `UnixStream` 导入；锚点是固定端点常量与帧上限。多一个字符都拿不到豁免，
/// 那时 `.write_all(` 与 `fs::` 留在余串里，由通用扫描判为越权写。
fn client_remainder(source: &str) -> String {
    let remainder = super::authority_tokens::checked_closed(
        source,
        &["protocol::LAUNCHER_SOCKET", "FRAME_MAX_BYTES"],
        &[
            ("UnixStream::connect(socket)", 1),
            ("stream.write_all(&frame)", 1),
            ("read_exact", 2),
            ("std::os::unix::net::UnixStream", 1),
        ],
    );
    socket_free(remainder)
}

/// 清单之外的任何端点记号都让通用扫描把它当成越权写。
fn socket_free(remainder: String) -> String {
    if SOCKET_TOKENS.iter().any(|token| remainder.contains(token)) {
        return format!("fs::unexpected_socket {remainder}");
    }
    remainder
}

#[cfg(test)]
mod tests {
    use crate::support::{AuthorityViolation, source_violations, workspace_root};
    use std::{fs, path::Path};

    const CRATE: &str = "studiolo-sync-transport";
    const CLIENT: &str = "crates/sync-transport/src/linux/client.rs";

    fn violates(path: &Path, source: &str) -> bool {
        source_violations(CRATE, path, source).contains(&AuthorityViolation::GenericWrite)
    }

    #[test]
    fn the_launcher_client_is_exempt_only_in_its_exact_shape() {
        let path = Path::new(CLIENT);
        let source = fs::read_to_string(workspace_root().join(path)).unwrap();
        assert!(source_violations(CRATE, path, &source).is_empty());
        for extra in [
            "std::fs::write(other, body)?;",
            "let other = UnixStream::connect(other)?;",
            "stream.write_all(&frame)?;",
            "let listener = UnixListener::bind(socket)?;",
            "Command::new(program);",
            "File::open(other)?;",
            "let mut extra = [0u8; 4]; stream.read_exact(&mut extra)?;",
            "let upstream = TcpStream::connect(address)?;",
        ] {
            assert!(violates(path, &format!("{source}\n{extra}")), "{extra}");
        }
        for (before, after) in [
            ("protocol::LAUNCHER_SOCKET", "\"/tmp/other.sock\""),
            ("FRAME_MAX_BYTES", "usize::MAX"),
            ("UnixStream::connect(socket)", "UnixStream::connect(&other)"),
        ] {
            assert!(source.contains(before), "{before}");
            assert!(violates(path, &source.replace(before, after)), "{before}");
        }
        // 同一份代码放到别的文件名下拿不到豁免；别的文件连端点记号都不许出现。
        assert!(violates(
            Path::new("crates/sync-transport/src/linux/other.rs"),
            &source
        ));
        assert!(violates(
            Path::new("crates/sync-transport/src/linux/backend.rs"),
            "let stream = UnixStream::connect(path);"
        ));
        assert!(violates(
            Path::new("crates/sync-transport/src/owned_run/owner.rs"),
            "let listener = UnixListener::bind(path);"
        ));
        assert!(!violates(
            Path::new("crates/sync-transport/src/owned_run/owner.rs"),
            "let value = 1;"
        ));
    }
}
