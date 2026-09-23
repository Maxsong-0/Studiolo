//! M5/M6 私有权限位置的精确清单；未匹配或重复的调用仍然禁止。
use std::path::Path;
pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-gateway" {
        return source.to_owned();
    }
    if path.ends_with("crates/gateway/src/relay_store.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "letparent=path.parent().ok_or(RelayError::Storage)?;",
                "metadata.nlink()!=1",
                "CREATETABLEIFNOTEXISTSrelay_grants",
                "UPDATErelay_settingsSETfrozen=1",
            ],
            &[
                ("fs::{self,File}", 1),
                ("std::os::unix::fs::{MetadataExt,OpenOptionsExt}", 1),
                ("fs::symlink_metadata(parent)", 1),
                ("fs::symlink_metadata(path)", 1),
                (
                    "File::options().write(true).create_new(true).mode(0o600).open(path)",
                    1,
                ),
                ("Connection::open(path)", 1),
            ],
        );
    }
    if path.ends_with("crates/gateway/src/durable_relay.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "letpath=self.raw_root.join(raw);",
                "letpath=self.raw_root.join(&name);",
                "\"{}-{}-{expires}.raw\"",
                "self.publisher.seal(grant,call,&path,&digest)?;",
            ],
            &[
                ("fs::{self,File}", 1),
                ("std::os::unix::fs::MetadataExt", 1),
                ("fs::symlink_metadata(&raw_root)", 1),
                ("fs::read(path)", 1),
                (
                    "File::options().write(true).create_new(true).open(&path)",
                    1,
                ),
                ("file.write_all(&bytes).and_then(|()|file.sync_all())", 1),
                ("File::open(&self.raw_root).and_then(|f|f.sync_all())", 1),
            ],
        );
    }
    if path.ends_with("crates/gateway/src/relay_retention.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "name.strip_suffix(\".raw\")",
                "ifnow>=deadline",
                "128*1024*1024",
            ],
            &[
                ("fs::read_dir(root)", 2),
                ("fs::symlink_metadata(entry.path())", 1),
                ("fs::remove_file(entry.path())", 1),
            ],
        );
    }
    if path.ends_with("crates/gateway/src/bin/gateway-model-relay.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "m.mode()&0o777!=0o600",
                "m.len()>16384",
                "config.client_uid==rustix::process::geteuid().as_raw()",
                "UnixListener::bind(&config.socket)",
            ],
            &[
                ("fs::{MetadataExt,PermissionsExt}", 1),
                ("fs::canonicalize(path)", 1),
                ("fs::symlink_metadata(path)", 1),
                ("fs::symlink_metadata(parent)", 1),
                ("fs::read(path)", 1),
                (
                    "fs::set_permissions(&config.socket,fs::Permissions::from_mode(0o660))",
                    1,
                ),
            ],
        );
    }
    if path.ends_with("crates/gateway/src/relay_remote.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "fnwrite_frame(stream:&mutUnixStream,bytes:&[u8])",
                "peer.uid.as_raw()!=expected_uid",
            ],
            &[(
                "stream.write_all(&size).and_then(|()|stream.write_all(bytes)).and_then(|()|stream.flush())",
                1,
            )],
        );
    }
    if path.ends_with("crates/gateway/src/relay_unix.rs") {
        return super::authority_tokens::checked(
            source,
            &[
                "mutstream:UnixStream",
                "relay.relay(&ticket,call,&path,&body)",
                "header.len()>16384",
            ],
            &[("stream.write_all(&body)", 1)],
        );
    }
    source.to_owned()
}
