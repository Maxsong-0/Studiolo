//! 因子网络动作不获得原始来源工厂、秘密环境或通用文件权限。

use crate::support::{rust_sources, source_violations, workspace_root};
use std::{fs, path::Path};

#[test]
fn unix_socket_adapter_does_not_authorize_other_file_or_database_writers() {
    let path = Path::new("crates/local-server/src/unix_ingress/layout.rs");
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations("studiolo-local-server", path, &source).is_empty());
    for extra in [
        "fs::write(path, bytes);",
        "File::create(path);",
        "open(other, flags, mode);",
        "unlinkat(root, unrelated, AtFlags::empty());",
    ] {
        assert!(
            !source_violations("studiolo-local-server", path, &format!("{source}\n{extra}"))
                .is_empty()
        );
    }
    assert!(
        !source_violations(
            "studiolo-local-server",
            Path::new("crates/local-server/src/handler.rs"),
            &source
        )
        .is_empty()
    );
    assert!(!source_violations("studiolo-auth", path, &source).is_empty());
}

#[test]
fn source_proofs_and_seed_environment_have_a_single_trusted_construction_boundary() {
    let root = workspace_root();
    for path in rust_sources(&root.join("crates/local-server/src")) {
        let source = fs::read_to_string(&path).unwrap();
        if !path.ends_with("startup/factor_input.rs") {
            assert!(
                !source.contains("std::env::var_os("),
                "环境只能在因子装配读取：{path:?}"
            );
        }
        if !path.ends_with("unix_ingress/peer.rs") {
            assert!(
                !source.contains("TrustedNetworkSource {"),
                "来源不能由普通处理器构造：{path:?}"
            );
            assert!(!source.contains("UnixPeer {"), "连接凭据构造边界：{path:?}");
        }
    }
    let peer =
        fs::read_to_string(root.join("crates/local-server/src/unix_ingress/peer.rs")).unwrap();
    assert!(peer.contains(".peer_cred()"));
    assert!(!peer.contains("pub fn source"));
    let input =
        fs::read_to_string(root.join("crates/local-server/src/startup/factor_input.rs")).unwrap();
    assert!(input.contains("TotpSeed::from_hex("));
    assert!(!input.contains("TotpEnrollment"));
}
