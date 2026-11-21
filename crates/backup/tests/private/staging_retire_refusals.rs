//! F16 修法的另一侧：已改名、删到一半的登记里出现任何不是本程序留下的样子，整轮拒绝、什么都不删；
//! 名字必须是程序写出来的那种规范写法，inode 要对得上，身份记录只能在别的都删完之后才缺。
use super::*;
use crate::checkpoint::staging::CleanupSummary;
use std::path::Path;

/// 已改名的登记里出现任何不是本程序留下的样子：整轮拒绝，什么都不删。
fn refused(f: &Fixture, expected: StagingError) {
    let before = f.layout();
    assert_eq!(
        reclaim_staging(&f.parent, f.reader, deadline()),
        Err(expected),
        "{before:?}"
    );
    assert_eq!(f.layout(), before);
}
/// 中断的尝试走到登记目录已改名、里面还剩 `left` 个文件（满的是 identity、export、pending 三份）。
fn interrupted_until(left: usize) -> (Fixture, PathBuf) {
    let f = Fixture::new(false);
    let area = f.area("interrupted");
    raw(&area);
    die(area);
    let path = f.removing_with(left);
    (f, path)
}
/// 到期的封存走到登记目录已改名、里面还剩 `left` 个文件（满的是四份）。
fn expired_until(left: usize) -> (Fixture, PathBuf) {
    let f = Fixture::new(false);
    sealed(&f, "expired");
    let path = f.removing_with(left);
    (f, path)
}
fn rename_to(path: &Path, name: String) -> PathBuf {
    let renamed = path.with_file_name(name);
    fs::rename(path, &renamed).unwrap();
    renamed
}
fn key_and_inode(path: &Path) -> (String, u64) {
    let name = path.file_name().unwrap().to_str().unwrap();
    let (key, inode) = journal::parse_removing(name).unwrap();
    assert_eq!(fs::metadata(path).unwrap().ino(), inode);
    (key.to_owned(), inode)
}
fn replace_json(path: &Path, change: impl FnOnce(&mut serde_json::Value)) {
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    change(&mut value);
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o400)).unwrap();
}
#[test]
fn emptied_registration_is_only_resumed_under_its_own_inode() {
    let (f, path) = interrupted_until(0);
    let (key, inode) = key_and_inode(&path);
    rename_to(&path, format!("removing-{key}-{}", inode + 1));
    refused(&f, StagingError::Changed);
}
#[test]
fn removing_name_must_be_the_canonical_form() {
    let names: [fn(&str, u64) -> String; 5] = [
        |key, inode| format!("removing-{key}-0{inode}"),
        |key, inode| format!("removing-{key}-+{inode}"),
        |key, _| format!("removing-{key}"),
        |key, inode| format!("removing-{}-{inode}", &key[1..]),
        |key, inode| format!("removing-{}-{inode}", key.to_uppercase()),
    ];
    for name in names {
        let (f, path) = interrupted_until(3);
        let (key, inode) = key_and_inode(&path);
        rename_to(&path, name(&key, inode));
        refused(&f, StagingError::CleanupRequired);
    }
}
#[test]
fn identity_may_only_be_missing_once_everything_else_is_gone() {
    let (f, path) = interrupted_until(3);
    fs::remove_file(path.join("identity.json")).unwrap();
    refused(&f, StagingError::CleanupRequired);
}
#[test]
fn foreign_or_damaged_records_in_a_removing_registration_are_refused() {
    let (f, path) = interrupted_until(3);
    private_file(&path.join("unknown"), b"do not touch");
    refused(&f, StagingError::Changed);
    let (f, path) = interrupted_until(3);
    replace_json(&path.join("identity.json"), |v| {
        v["parent"][1] = serde_json::json!(0)
    });
    refused(&f, StagingError::Changed);
    let (f, path) = interrupted_until(2);
    fs::set_permissions(path.join("pending.json"), fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(path.join("pending.json"), b"{").unwrap();
    fs::set_permissions(path.join("pending.json"), fs::Permissions::from_mode(0o400)).unwrap();
    refused(&f, StagingError::Changed);
    // 身份记录还在时，封存记录也要对得上它。
    let (f, path) = expired_until(4);
    replace_json(&path.join("sealed.json"), |v| {
        v["reference"]["checkpoint_id"] = serde_json::json!("someone-else")
    });
    refused(&f, StagingError::Changed);
}
#[test]
fn removing_registration_never_vouches_for_roots_or_a_second_registration() {
    for sibling in ["export", "pending", "sealed", "retained"] {
        let (f, path) = interrupted_until(3);
        let (key, _) = key_and_inode(&path);
        let root = f.parent.join(format!("{sibling}-{key}"));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        private_file(&root.join("keep"), b"not ours");
        refused(&f, StagingError::CleanupRequired);
    }
}
#[test]
fn new_attempts_wait_until_a_half_removed_registration_is_finished() {
    let (f, _) = interrupted_until(2);
    assert_eq!(
        f.try_area("next").err(),
        Some(StagingError::CleanupRequired)
    );
    let summary = reclaim_staging(&f.parent, f.reader, deadline()).unwrap();
    assert_eq!(
        summary,
        CleanupSummary {
            resumed: 1,
            ..CleanupSummary::default()
        }
    );
    f.area("next").abort().unwrap();
}
/// 同一进程里收尾还没开始删登记，就出现了带自己 inode 的 `removing-` 名字：不是本程序改的，拒绝，什么都不删。
#[test]
fn cleanup_refuses_a_removing_name_it_did_not_create() {
    let f = Fixture::new(false);
    let mut area = f.area("early");
    raw(&area);
    let marker = f.parent.join(&area.lifecycle.name);
    let key = area.lifecycle.name.strip_prefix("incomplete-").unwrap();
    let inode = fs::metadata(&marker).unwrap().ino();
    rename_to(&marker, journal::removing_name(key, (0, inode)));
    let before = f.layout();
    assert_eq!(area.cleanup(), Err(StagingError::CleanupFailed));
    assert_eq!(f.layout(), before);
    die(area);
}
/// 同一进程重试时，改了名的登记里身份记录没了、却还剩别的文件：不是本程序删出来的样子，拒绝，剩下的不动。
#[test]
fn cleanup_retry_refuses_a_removing_marker_missing_identity_with_files_left() {
    for steps in 0..10_000 {
        let f = Fixture::new(false);
        let mut area = f.area("retried");
        raw(&area);
        let _ = area.cleanup_within(&mut Budget::steps(deadline(), steps));
        let Some(path) = f.removing() else {
            die(area);
            continue;
        };
        assert_eq!(fs::read_dir(&path).unwrap().count(), 3, "{:?}", f.layout());
        fs::remove_file(path.join("identity.json")).unwrap();
        let before = f.layout();
        assert_eq!(area.cleanup(), Err(StagingError::CleanupFailed));
        assert_eq!(f.layout(), before);
        die(area);
        return;
    }
    panic!("一万步都没走到改名之后");
}
fn private_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
