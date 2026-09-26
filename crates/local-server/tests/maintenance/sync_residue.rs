//! 残留清除的目录遍历契约（不依赖 W4 的持久状态）：先整棵核验再删，链接一律拒绝且一个字节不动，
//! 计数精确到文件与目录（顶层文件夹计入目录数），目标缺失即「没有残留可清」。

use super::super::{
    MaintenanceError,
    sync::{Counts, locate, sweep},
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

struct View {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

impl View {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let root = temp.path().to_path_buf();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        Self { _temp: temp, root }
    }

    fn dir(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn file(&self, name: &str) {
        fs::write(self.root.join(name), "残留").unwrap();
    }

    fn run(&self, folder: &str, delete: bool) -> Result<(usize, usize), MaintenanceError> {
        let (parent, name) = locate(&self.root, folder)?;
        let mut counts = Counts::default();
        sweep(&parent, &name, 0, &mut counts, delete)?;
        Ok((counts.files, counts.directories))
    }
}

#[test]
fn walker_counts_then_removes_exactly_the_tree_and_leaves_siblings_alone() {
    let v = View::new();
    v.dir("School");
    v.dir("School/sub");
    v.dir("School/sub/deeper");
    v.file("School/a.md");
    v.file("School/sub/b.md");
    v.file("School/sub/deeper/c.txt");
    v.dir("笔记");
    v.file("笔记/keep.md");
    // 核验一趟只计数，不删。
    assert_eq!(v.run("School", false).unwrap(), (3, 3));
    assert!(v.root.join("School/sub/deeper/c.txt").is_file());
    assert_eq!(v.run("School", true).unwrap(), (3, 3));
    assert!(!v.root.join("School").exists());
    assert!(v.root.join("笔记/keep.md").is_file());
    // 目标不存在＝没有残留可清；嵌套名逐段打开，中间段缺失同样如此。
    assert!(matches!(
        v.run("School", false),
        Err(MaintenanceError::SyncResidue)
    ));
    assert!(matches!(
        v.run("Missing/inner", false),
        Err(MaintenanceError::SyncResidue)
    ));
    // 嵌套名只删末段那一棵，父目录保留。
    v.dir("a");
    v.dir("a/b");
    v.file("a/b/x.md");
    assert_eq!(v.run("a/b", true).unwrap(), (1, 1));
    assert!(v.root.join("a").is_dir() && !v.root.join("a/b").exists());
}

#[test]
fn walker_refuses_links_and_irregular_objects_before_deleting_anything() {
    let v = View::new();
    v.dir("School");
    v.dir("School/sub");
    v.file("School/a.md");
    symlink("a.md", v.root.join("School/sub/link.md")).unwrap();
    for delete in [false, true] {
        assert!(matches!(
            v.run("School", delete),
            Err(MaintenanceError::SyncResidueUnsafe)
        ));
    }
    assert!(v.root.join("School/a.md").is_file());
    assert!(v.root.join("School/sub/link.md").is_symlink());
    // 文件夹本身是链接：不跟随，链接指向的目录一个字节不动。
    let elsewhere = v.dir("elsewhere");
    fs::write(elsewhere.join("x.md"), "x").unwrap();
    symlink(&elsewhere, v.root.join("Linked")).unwrap();
    assert!(matches!(
        v.run("Linked", true),
        Err(MaintenanceError::SyncResidueUnsafe)
    ));
    assert!(elsewhere.join("x.md").is_file());
    // 顶层是普通文件而非目录：不是文件夹残留，同样拒绝。
    v.file("Note");
    assert!(matches!(
        v.run("Note", true),
        Err(MaintenanceError::SyncResidueUnsafe)
    ));
    assert!(v.root.join("Note").is_file());
    // 视图根自己是链接也打不开。
    let link_root = v.root.join("view-link");
    symlink(&v.root, &link_root).unwrap();
    assert!(matches!(
        locate(&link_root, "School"),
        Err(MaintenanceError::SyncResidueUnsafe)
    ));
}
