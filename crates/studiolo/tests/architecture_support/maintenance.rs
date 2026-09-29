//! 离线事务维护只得到固定控制 TTY；不能取得任意文件或子进程权限。

use std::path::Path;

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-local-server" {
        return source.to_owned();
    }
    if path.ends_with("crates/local-server/src/maintenance/sync_rebind.rs") {
        // 离线重绑定只探测既有 Plane 固定文件，不授予初始化或目录清理能力。
        return super::authority_tokens::checked_closed(
            source,
            &[
                "PLANE_LOCK",
                "PLANE_STATE_FILE",
                "TransportRebind::open_existing",
                "StageStore::open_existing_for_maintenance",
            ],
            &[("std::fs::symlink_metadata(plane_root.join(name))", 1)],
        );
    }
    if path.ends_with("crates/local-server/src/maintenance/sync.rs") {
        // T-132 P1（B 轮 W6）：`sync-init` 只建 `<plane_root>/bridge` 0700 与探测四项是否已存在；
        // `sync-purge-residue` 只在镜像视图根下按目录逐段 NOFOLLOW 进入、链接即拒、只删普通文件与空目录。
        return super::authority_tokens::checked_closed(
            source,
            &[
                "BRIDGE_DIR",
                "STAGE_DIR",
                "PLANE_STATE_FILE",
                "DIRECTORY_FLAGS",
            ],
            &[
                (
                    "userustix::fs::{AtFlags,Dir,FileType,Mode,OFlags,open,openat,statat,unlinkat};",
                    1,
                ),
                (
                    "usestd::{ffi::CString,fs,os::fd::OwnedFd,os::unix::fs::DirBuilderExt,path::Path};",
                    1,
                ),
                ("fs::symlink_metadata(plane_root.join(name))", 1),
                ("fs::DirBuilder::new().mode(0o700).create(&bridge)", 1),
                ("open(view_root,DIRECTORY_FLAGS,Mode::empty())", 1),
                ("openat(&current,&name,DIRECTORY_FLAGS,Mode::empty())", 1),
                ("openat(parent,name,DIRECTORY_FLAGS,Mode::empty())", 1),
                ("statat(parent,name,AtFlags::SYMLINK_NOFOLLOW)", 1),
                ("Dir::read_from(&directory)", 1),
                ("unlinkat(parent,name,AtFlags::REMOVEDIR)", 1),
                ("unlinkat(parent,name,AtFlags::empty())", 1),
            ],
        );
    }
    if !path.ends_with("crates/local-server/src/maintenance/terminal.rs") {
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

#[cfg(test)]
mod tests {
    use crate::support::{AuthorityViolation, source_violations, workspace_root};
    use std::{fs, path::Path};

    const CRATE: &str = "studiolo-local-server";
    const SYNC: &str = "crates/local-server/src/maintenance/sync.rs";

    fn violates(path: &str, source: &str) -> bool {
        source_violations(CRATE, Path::new(path), source)
            .contains(&AuthorityViolation::GenericWrite)
    }

    #[test]
    fn rebind_allows_only_fixed_file_presence_probe() {
        let path = "crates/local-server/src/maintenance/sync_rebind.rs";
        let source = fs::read_to_string(workspace_root().join(path)).unwrap();
        assert!(!violates(path, &source));
        for extra in [
            "std::fs::write(other, body)?;",
            "std::fs::symlink_metadata(other)?;",
            "std::fs::symlink_metadata(plane_root.join(name))?;",
        ] {
            assert!(violates(path, &format!("{source}\n{extra}")));
        }
        assert!(violates(path, &source.replace("PLANE_LOCK", "OTHER")));
        assert!(violates(
            "crates/local-server/src/maintenance/other.rs",
            &source
        ));
    }

    fn sync_fixture() -> String {
        [
            "use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, open, openat, statat, unlinkat};",
            "use std::{ffi::CString, fs, os::fd::OwnedFd, os::unix::fs::DirBuilderExt, path::Path};",
            "const BRIDGE_DIR: &str = \"bridge\"; const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY;",
            "let _ = (STAGE_DIR, PLANE_STATE_FILE);",
            "let present = fs::symlink_metadata(plane_root.join(name)).is_ok();",
            "fs::DirBuilder::new().mode(0o700).create(&bridge)?;",
            "let root: OwnedFd = open(view_root, DIRECTORY_FLAGS, Mode::empty())?;",
            "let next = openat(&current, &name, DIRECTORY_FLAGS, Mode::empty())?;",
            "let child = openat(parent, name, DIRECTORY_FLAGS, Mode::empty())?;",
            "let stat = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;",
            "for entry in Dir::read_from(&directory)? { let _ = FileType::Directory; let _ = CString::new(\"\"); }",
            "unlinkat(parent, name, AtFlags::REMOVEDIR)?;",
            "unlinkat(parent, name, AtFlags::empty())?;",
        ]
        .join("\n")
    }

    #[test]
    fn the_sync_maintenance_file_is_exempt_only_in_its_registered_shape() {
        let fixture = sync_fixture();
        assert!(!violates(SYNC, &fixture), "登记形态本身不该红");
        for extra in [
            "std::fs::write(other, body)?;",
            "File::create(other)?;",
            "Command::new(program);",
            "unlinkat(parent, other, AtFlags::empty())?;",
            "let more = open(other, DIRECTORY_FLAGS, Mode::empty())?;",
        ] {
            assert!(violates(SYNC, &format!("{fixture}\n{extra}")), "{extra}");
        }
        assert!(violates(SYNC, &fixture.replace("BRIDGE_DIR", "OTHER")));
        assert!(violates(
            "crates/local-server/src/maintenance/workflow.rs",
            &fixture
        ));
        let source = fs::read_to_string(workspace_root().join(SYNC)).unwrap();
        assert!(source_violations(CRATE, Path::new(SYNC), &source).is_empty());
    }
}
