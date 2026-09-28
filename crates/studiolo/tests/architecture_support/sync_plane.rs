//! Bridge 驱动平面（T-132 P1，ADR-025 D8／D9）在 local-server 里的真实 IO 只许在三个固定文件：
//! `sync_plane/mirror/stage.rs`（外 stage：objects/<sha256>、index.json、接收临时件）、
//! `sync_plane/recovery.rs`（plane-state.json、plane.lock、残留守卫）与 `sync_plane/mirror.rs`
//! （视图观察只读；安装只许 stage 对象 renameat 进视图；删除前保全）。固定名常量不在场就拿不到
//! 豁免；rustix 调用按 W4 任务书第 2 节的固定形态精确计数，导入表只能含类型与登记过的调用名。

use std::path::Path;

/// 任何登记文件的导入表都可以带的名字：rustix 类型名，加只读的 `fstat`（读元数据不是写能力，不计数；
/// `mirror.rs` 按 W4 清单另外精确计数）。
const RUSTIX_IMPORTABLE: [&str; 10] = [
    "AtFlags",
    "CWD",
    "Dir",
    "FileType",
    "FlockOperation",
    "Mode",
    "OFlags",
    "Stat",
    "StatxFlags",
    "fstat",
];

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-local-server" {
        return source.to_owned();
    }
    if path.ends_with("crates/local-server/src/sync_plane/mirror/stage.rs") {
        // B 轮 W4 落地形态：唯一打开点 open_with（openat）＋fstat；flush／rename_within／remove_at／
        // make_directory_at／entry_names 各是一个只认已有描述符的助手，每种调用只出现一次。
        return plane_remainder(
            source,
            &[
                "STAGE_DIR",
                "OBJECTS_DIR",
                "INDEX_FILE",
                "INDEX_STAGE_FILE",
                "RECEIVING_FILE",
                "RETENTION_SECONDS",
            ],
            &[
                ("openat(", 1),
                ("fstat(", 1),
                ("fsync(", 1),
                ("renameat(", 1),
                ("unlinkat(", 1),
                ("mkdirat(", 1),
                ("Dir::read_from(", 1),
                ("statx(", 2),
            ],
        );
    }
    if path.ends_with("crates/local-server/src/sync_plane/recovery.rs") {
        // B 轮 W4 落地形态：directory_path 从 CWD 逐段 openat＋fstat；PlaneStateStore::open 一次 flock、
        // Drop 一次 flock(Unlock)（并发 fork 继承描述符时立即结束租约）；
        // 建文件／整读／原子替换全部借 stage.rs 的描述符助手，本文件不再有 renameat／fsync。
        return plane_remainder(
            source,
            &["PLANE_STATE_FILE", "PLANE_STATE_STAGE_FILE", "PLANE_LOCK"],
            &[("openat(", 1), ("fstat(", 1), ("flock(", 2)],
        );
    }
    if path.ends_with("crates/local-server/src/sync_plane/mirror.rs") {
        // B 轮 W4 落地形态：观察与删除都经 stage.rs 的描述符助手；本文件只剩 install 的
        // renameat（stage 对象 → 视图目标）与父目录 fsync 各一次。
        return plane_remainder(
            source,
            &["MirrorView", "renameat"],
            &[("renameat(", 1), ("fsync(", 1)],
        );
    }
    source.to_owned()
}

/// 计数在去掉公开签名 `fn open(` 之后做（`StageStore::open`／`PlaneStateStore::open` 的定义不是调用）；
/// 计数与锚点全部成立时才放行唯一一条 `use rustix::fs::{…};`（花括号里只能是类型与登记过的调用名）
/// 与 `rustix::fs::Dir` 路径引用，再交给通用闭合检查清掉调用。任一不成立就原样返回，
/// 导入里的 `fs::` 会让通用扫描判为越权。
fn plane_remainder(source: &str, anchors: &[&str], calls: &[(&str, usize)]) -> String {
    let compact = super::authority_tokens::compact(source);
    let counted = compact.replace("fnopen(", "");
    let exact = anchors.iter().all(|anchor| compact.contains(anchor))
        && calls
            .iter()
            .all(|(call, count)| counted.matches(call).count() == *count);
    let allowed = calls
        .iter()
        .map(|(call, _)| call.trim_end_matches('('))
        .collect::<Vec<_>>();
    let Some(stripped) = exact
        .then(|| without_rustix_import(&counted, &allowed))
        .flatten()
    else {
        return compact;
    };
    super::authority_tokens::checked_closed(
        &stripped.replace("rustix::fs::Dir", ""),
        anchors,
        calls,
    )
}

/// 去掉唯一一条 `use rustix::fs::{…};`；名字越出白名单或还有第二条导入时返回 `None`。
fn without_rustix_import(compact: &str, allowed_calls: &[&str]) -> Option<String> {
    const PREFIX: &str = "userustix::fs::{";
    let start = compact.find(PREFIX)?;
    let body_start = start + PREFIX.len();
    let length = compact[body_start..].find("};")?;
    let names = &compact[body_start..body_start + length];
    if names
        .split(',')
        .any(|name| !RUSTIX_IMPORTABLE.contains(&name) && !allowed_calls.contains(&name))
    {
        return None;
    }
    let stripped = format!(
        "{}{}",
        &compact[..start],
        &compact[body_start + length + 2..]
    );
    (!stripped.contains(PREFIX)).then_some(stripped)
}

#[cfg(test)]
mod tests {
    use crate::support::{AuthorityViolation, source_violations, workspace_root};
    use std::{fs, path::Path};

    const CRATE: &str = "studiolo-local-server";
    const STAGE: &str = "crates/local-server/src/sync_plane/mirror/stage.rs";
    const RECOVERY: &str = "crates/local-server/src/sync_plane/recovery.rs";
    const MIRROR: &str = "crates/local-server/src/sync_plane/mirror.rs";

    fn violates(path: &str, source: &str) -> bool {
        source_violations(CRATE, Path::new(path), source)
            .contains(&AuthorityViolation::GenericWrite)
    }

    fn stage_fixture() -> String {
        [
            "use rustix::fs::{AtFlags, CWD, Dir, FileType, Mode, OFlags, Stat, StatxFlags, fstat, fsync, mkdirat, openat, renameat, statx, unlinkat};",
            "let first = statx(left, \"\", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)?; let second = statx(right, \"\", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)?;",
            "pub const STAGE_DIR: &str = \"stage\"; pub const OBJECTS_DIR: &str = \"objects\";",
            "pub const INDEX_FILE: &str = \"index.json\"; pub const INDEX_STAGE_FILE: &str = \"index.json.new\";",
            "pub const RECEIVING_FILE: &str = \"receiving.partial\"; pub const RETENTION_SECONDS: u64 = 604_800;",
            "fn open_with(dir: BorrowedFd<'_>, name: &str, flags: OFlags, mode: Mode) -> io::Result<(OwnedFd, Stat)> {",
            "let fd = openat(dir, name, flags | OFlags::NOFOLLOW | OFlags::CLOEXEC, mode)?; let stat = fstat(&fd)?; Ok((fd, stat)) }",
            "fn flush(fd: BorrowedFd<'_>) -> io::Result<()> { fsync(fd)?; Ok(()) }",
            "fn rename_within(dir: BorrowedFd<'_>, from: &str, to: &str) -> io::Result<()> { renameat(dir, from, dir, to)?; Ok(()) }",
            "fn remove_at(dir: BorrowedFd<'_>, name: &str) -> io::Result<bool> { unlinkat(dir, name, AtFlags::empty())?; Ok(true) }",
            "fn make_directory_at(dir: BorrowedFd<'_>, name: &str) -> io::Result<()> { mkdirat(dir, name, Mode::from_raw_mode(0o700))?; Ok(()) }",
            "fn entry_names(dir: BorrowedFd<'_>) -> io::Result<Vec<OsString>> { for entry in Dir::read_from(dir)? { let _ = FileType::RegularFile; } Ok(Vec::new()) }",
            "let _ = (CWD, RETENTION_SECONDS);",
        ]
        .join("\n")
    }

    fn recovery_fixture() -> String {
        [
            "use rustix::fs::{CWD, FlockOperation, Mode, OFlags, Stat, flock, fstat, openat};",
            "pub const PLANE_STATE_FILE: &str = \"plane-state.json\"; pub const PLANE_LOCK: &str = \"plane.lock\";",
            "pub const PLANE_STATE_STAGE_FILE: &str = \"plane-state.json.new\";",
            "fn directory_path(path: &Path) -> io::Result<(OwnedFd, Stat)> {",
            "let mut current = CWD; let fd = openat(current, segment, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())?;",
            "let stat = fstat(&fd)?; Ok((fd, stat)) }",
            "pub fn open(plane_root: &Path) -> Result<Self, PlaneError> {",
            "let (lock, _) = stage::create_at(root.as_fd(), PLANE_LOCK, false)?;",
            "flock(&lock, FlockOperation::NonBlockingLockExclusive)?;",
            "impl Drop for PlaneStateStore { fn drop(&mut self) { let _ = flock(&self.lease, FlockOperation::Unlock); } }",
            "let state = stage::read_fully(root.as_fd(), PLANE_STATE_FILE)?; stage::replace_file(root.as_fd(), PLANE_STATE_STAGE_FILE, PLANE_STATE_FILE, &bytes)?;",
        ]
        .join("\n")
    }

    fn mirror_fixture() -> String {
        [
            "use rustix::fs::{fsync, renameat};",
            "pub struct MirrorView { root: PathBuf, identity: (u64, u64) }",
            "let (target, current) = stage::open_regular_at(view.as_fd(), path, euid, max)?;",
            "stage::make_directory_at(view.as_fd(), parent, false)?;",
            "renameat(objects.as_fd(), hex, parent_directory.as_fd(), target_name)?; fsync(parent_directory.as_fd())?;",
            "stage::remove_at(view.as_fd(), target_name)?;",
        ]
        .join("\n")
    }

    #[test]
    fn each_plane_io_file_is_exempt_only_in_its_registered_shape() {
        for (path, fixture, duplicated) in [
            (
                STAGE,
                stage_fixture(),
                "let more = openat(dir, other, flags, mode)?;",
            ),
            (
                RECOVERY,
                recovery_fixture(),
                "flock(&other, FlockOperation::LockExclusive)?;",
            ),
            (
                MIRROR,
                mirror_fixture(),
                "renameat(objects.as_fd(), hex, &view, other)?;",
            ),
        ] {
            assert!(!violates(path, &fixture), "{path} 的登记形态本身不该红");
            for extra in [
                "std::fs::write(other, body)?;",
                "File::create(other)?;",
                "file.write_all(body)?;",
                "Command::new(program);",
                "let ambient = rustix::fs::open(other, OFlags::RDWR, Mode::empty())?;",
                "use rustix::fs::{symlinkat};",
                "use rustix::fs::{Mode};",
                duplicated,
            ] {
                assert!(
                    violates(path, &format!("{fixture}\n{extra}")),
                    "{path}: {extra}"
                );
            }
            // 同一份代码放到未登记的文件名下：一律红。
            assert!(
                violates("crates/local-server/src/sync_plane/driver.rs", &fixture),
                "{path}"
            );
        }
        for (path, fixture, anchor) in [
            (STAGE, stage_fixture(), "RECEIVING_FILE"),
            (RECOVERY, recovery_fixture(), "PLANE_LOCK"),
            (MIRROR, mirror_fixture(), "MirrorView"),
        ] {
            assert!(fixture.contains(anchor), "{path}: {anchor}");
            assert!(
                violates(path, &fixture.replace(anchor, "OTHER")),
                "{path}: {anchor}"
            );
        }
        // 导入表是本文件能用的 rustix 调用闭集：多导入一个未登记的调用名就红。
        let widened = stage_fixture().replace("mkdirat, openat,", "mkdirat, openat, symlinkat,");
        assert!(widened.contains("symlinkat"));
        assert!(violates(STAGE, &widened));
        // 当前工作树里三个登记文件与几个纯逻辑子模块都必须过守卫。
        let root = workspace_root();
        for path in [
            STAGE,
            RECOVERY,
            MIRROR,
            "crates/local-server/src/sync_plane/mirror/stage/store.rs",
            "crates/local-server/src/sync_plane/mirror/stage/index.rs",
            "crates/local-server/src/sync_plane/mirror/content.rs",
            "crates/local-server/src/sync_plane/mirror/observe.rs",
            "crates/local-server/src/sync_plane/driver/content.rs",
        ] {
            let source = fs::read_to_string(root.join(path)).unwrap();
            assert!(
                source_violations(CRATE, Path::new(path), &source).is_empty(),
                "{path}"
            );
        }
    }
}
