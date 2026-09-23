//! Bridge 驱动平面（T-132 P1，ADR-025 D8／D9）在 local-server 里的真实 IO 只许在两个固定文件：
//! `sync_plane/mirror/stage.rs`（外 stage：objects/<sha256>、index.json、接收临时件）与
//! `sync_plane/recovery.rs`（plane-state.json）。固定名常量不在场就拿不到豁免；
//! 镜像视图的观察／安装（`mirror.rs`）只许经 stage 对象 renameat 进视图，同样按固定形态登记。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-local-server" {
        return source.to_owned();
    }
    if path.ends_with("crates/local-server/src/sync_plane/mirror/stage.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "STAGE_DIR",
                "OBJECTS_DIR",
                "INDEX_FILE",
                "INDEX_STAGE_FILE",
                "RECEIVING_FILE",
                "RETENTION_SECONDS",
            ],
            &[],
        );
    }
    if path.ends_with("crates/local-server/src/sync_plane/recovery.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &["PLANE_STATE_FILE", "PLANE_STATE_STAGE_FILE", "PLANE_LOCK"],
            &[],
        );
    }
    if path.ends_with("crates/local-server/src/sync_plane/mirror.rs") {
        return super::authority_tokens::checked_closed(source, &["MirrorView", "renameat"], &[]);
    }
    source.to_owned()
}
