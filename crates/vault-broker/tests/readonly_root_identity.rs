//! 启动方比较的是已固定的根句柄，不是稍后重开的同名目录。

#[test]
fn pinned_identity_survives_a_selected_path_replacement_without_following_it() {
    use std::{fs, os::unix::fs::MetadataExt};
    use studiolo_vault_broker::ReadOnlyVault;
    let temp = tempfile::tempdir().unwrap();
    let selected = temp.path().join("selected");
    fs::create_dir(&selected).unwrap();
    let before = fs::metadata(&selected).unwrap();
    let reader = ReadOnlyVault::open(&selected).unwrap();
    assert_eq!(
        reader.root_identity().unwrap(),
        (before.dev(), before.ino())
    );
    fs::rename(&selected, temp.path().join("retained")).unwrap();
    fs::create_dir(&selected).unwrap();
    let replacement = fs::metadata(&selected).unwrap();
    assert_ne!(
        reader.root_identity().unwrap(),
        (replacement.dev(), replacement.ino())
    );
    assert_eq!(
        reader.root_identity().unwrap(),
        (before.dev(), before.ino())
    );
}
