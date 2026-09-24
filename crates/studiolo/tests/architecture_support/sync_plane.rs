//! Bridge 驱动平面（T-132 P1，ADR-025 D8／D9）在 local-server 里的真实 IO 只许在三个固定文件：
//! `sync_plane/mirror/stage.rs`（外 stage：objects/<sha256>、index.json、接收临时件）、
//! `sync_plane/recovery.rs`（plane-state.json、plane.lock、残留守卫）与 `sync_plane/mirror.rs`
//! （视图观察只读；安装只许 stage 对象 renameat 进视图；删除前保全）。固定名常量不在场就拿不到
//! 豁免；rustix 调用按 W4 任务书第 2 节的固定形态精确计数，导入表只能含类型与登记过的调用名。

use std::path::Path;

/// 任何登记文件的导入表都可以带的名字：rustix 类型名，加只读的 `fstat`（读元数据不是写能力，不计数；
/// `mirror.rs` 按 W4 清单另外精确计数）。
const RUSTIX_IMPORTABLE: [&str; 7] = [
    "AtFlags",
    "Dir",
    "FileType",
    "FlockOperation",
    "Mode",
    "OFlags",
    "fstat",
];

pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-local-server" {
        return source.to_owned();
    }
    if path.ends_with("crates/local-server/src/sync_plane/mirror/stage.rs") {
        // open 根目录一次；objects／index／receiving／index 暂存件各 openat 一次；接收封口与索引替换各
        // renameat 一次；孤儿、旧接收件、到期对象、扫描孤儿各 unlinkat 一次；建目录两次；fsync 四次。
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
                ("open(", 1),
                ("openat(", 4),
                ("renameat(", 2),
                ("unlinkat(", 4),
                ("mkdirat(", 2),
                ("fsync(", 4),
            ],
        );
    }
    if path.ends_with("crates/local-server/src/sync_plane/recovery.rs") {
        // open 平面根一次；lock／state／暂存件／残留文件夹各 openat 一次；flock 一次；原子替换一次；fsync 两次。
        return plane_remainder(
            source,
            &["PLANE_STATE_FILE", "PLANE_STATE_STAGE_FILE", "PLANE_LOCK"],
            &[
                ("open(", 1),
                ("openat(", 4),
                ("flock(", 1),
                ("renameat(", 1),
                ("fsync(", 2),
            ],
        );
    }
    if path.ends_with("crates/local-server/src/sync_plane/mirror.rs") {
        // 观察只读：openat＋fstat 各两次（遍历与复验）；安装：renameat／mkdirat／fsync 各一次；删除：unlinkat 一次。
        return plane_remainder(
            source,
            &["MirrorView", "renameat"],
            &[
                ("openat(", 2),
                ("fstat(", 2),
                ("renameat(", 1),
                ("mkdirat(", 1),
                ("fsync(", 1),
                ("unlinkat(", 1),
            ],
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
            "use rustix::fs::{AtFlags, Dir, Mode, OFlags, fstat, fsync, mkdirat, open, openat, renameat, unlinkat};",
            "pub const STAGE_DIR: &str = \"stage\"; pub const OBJECTS_DIR: &str = \"objects\";",
            "pub const INDEX_FILE: &str = \"index.json\"; pub const INDEX_STAGE_FILE: &str = \"index.json.new\";",
            "pub const RECEIVING_FILE: &str = \"receiving.partial\"; pub const RETENTION_SECONDS: u64 = 604_800;",
            "pub fn open(plane_root: &Path) -> Result<Self, PlaneError> {",
            "let stage = open(plane_root.join(STAGE_DIR), OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())?;",
            "let objects = openat(&stage, OBJECTS_DIR, FLAGS, Mode::empty())?;",
            "let index = openat(&stage, INDEX_FILE, FLAGS, Mode::empty())?;",
            "let stat = fstat(&index)?; for entry in Dir::read_from(&objects)? { unlinkat(&objects, name, AtFlags::empty())?; }",
            "mkdirat(&root, STAGE_DIR, Mode::from_raw_mode(0o700))?; mkdirat(&root, OBJECTS_DIR, Mode::from_raw_mode(0o700))?;",
            "let receiving = openat(&objects, RECEIVING_FILE, WRITE_FLAGS, Mode::from_raw_mode(0o600))?;",
            "fsync(&receiving)?; renameat(&objects, RECEIVING_FILE, &objects, hex)?; fsync(&objects)?;",
            "unlinkat(&objects, RECEIVING_FILE, AtFlags::empty())?;",
            "let staged = openat(&stage, INDEX_STAGE_FILE, WRITE_FLAGS, Mode::from_raw_mode(0o600))?;",
            "fsync(&staged)?; renameat(&stage, INDEX_STAGE_FILE, &stage, INDEX_FILE)?; fsync(&stage)?;",
            "unlinkat(&objects, expired, AtFlags::empty())?; unlinkat(&objects, orphan, AtFlags::empty())?;",
            "let _ = RETENTION_SECONDS;",
        ]
        .join("\n")
    }

    fn recovery_fixture() -> String {
        [
            "use rustix::fs::{FlockOperation, Mode, OFlags, flock, fsync, open, openat, renameat};",
            "pub const PLANE_STATE_FILE: &str = \"plane-state.json\"; pub const PLANE_LOCK: &str = \"plane.lock\";",
            "pub const PLANE_STATE_STAGE_FILE: &str = \"plane-state.json.new\";",
            "pub fn open(plane_root: &Path) -> Result<Self, PlaneError> {",
            "let root = open(plane_root, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())?;",
            "let lock = openat(&root, PLANE_LOCK, OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW, Mode::from_raw_mode(0o600))?;",
            "flock(&lock, FlockOperation::NonBlockingLockExclusive)?;",
            "let state = openat(&root, PLANE_STATE_FILE, READ_FLAGS, Mode::empty())?;",
            "let staged = openat(&root, PLANE_STATE_STAGE_FILE, WRITE_FLAGS, Mode::from_raw_mode(0o600))?;",
            "fsync(&staged)?; renameat(&root, PLANE_STATE_STAGE_FILE, &root, PLANE_STATE_FILE)?; fsync(&root)?;",
            "let folder = openat(&view, name, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW, Mode::empty())?;",
            "let mut entries = rustix::fs::Dir::read_from(&folder)?;",
        ]
        .join("\n")
    }

    fn mirror_fixture() -> String {
        [
            "use rustix::fs::{AtFlags, Mode, OFlags, fstat, fsync, mkdirat, openat, renameat, unlinkat};",
            "pub struct MirrorView { root: PathBuf, identity: (u64, u64) }",
            "let file = openat(&directory, name, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())?;",
            "let stat = fstat(&file)?; for entry in rustix::fs::Dir::read_from(&directory)? { }",
            "let target = openat(&view, path, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())?;",
            "let current = fstat(&target)?;",
            "mkdirat(&view, parent, Mode::from_raw_mode(0o700))?;",
            "renameat(&objects, hex, &view, target_path)?; fsync(&parent_directory)?;",
            "unlinkat(&view, target_path, AtFlags::empty())?;",
        ]
        .join("\n")
    }

    #[test]
    fn each_plane_io_file_is_exempt_only_in_its_registered_shape() {
        for (path, fixture, duplicated) in [
            (
                STAGE,
                stage_fixture(),
                "let more = openat(&stage, other, FLAGS, Mode::empty())?;",
            ),
            (
                RECOVERY,
                recovery_fixture(),
                "flock(&other, FlockOperation::LockExclusive)?;",
            ),
            (
                MIRROR,
                mirror_fixture(),
                "renameat(&objects, hex, &view, other)?;",
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
        let widened = stage_fixture().replace("mkdirat, open,", "mkdirat, open, symlinkat,");
        assert!(widened.contains("symlinkat"));
        assert!(violates(STAGE, &widened));
        // 当前工作树里的三个骨架文件也必须过守卫（还没有 IO 就不该红）。
        let root = workspace_root();
        for path in [STAGE, RECOVERY, MIRROR] {
            let source = fs::read_to_string(root.join(path)).unwrap();
            assert!(
                source_violations(CRATE, Path::new(path), &source).is_empty(),
                "{path}"
            );
        }
    }
}
