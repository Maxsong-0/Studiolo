//! 恢复契约：配置或根身份漂移后，既有能力必须立刻失效，不留部分可用状态。

use super::fixture::{Fixture, directory};
use serde_json::json;
use std::{fs, os::unix::fs::MetadataExt, path::Path};

#[test]
fn capability_revalidation_fails_after_the_private_config_content_changes() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();
    assert_eq!(approved.revalidate(), Ok(()));

    let mut changed = fixture.value.clone();
    changed["sync_scope"]["scope_epoch"] = json!(2);
    fixture.write(&changed);

    assert!(approved.revalidate().is_err());
    assert!(scope.revalidate().is_err());
}

/// 删掉被批准的根、原地新建同名目录：Linux 会把同一个 inode 号立刻发还，
/// 只比 `(dev, ino)` 的旧实现在这里完全看不出替换。断言写的是契约本身
/// ——旧能力必须死透——不依赖操作系统是否复用 inode 号。
#[test]
fn capability_revalidation_fails_after_the_private_root_object_is_replaced() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();

    fs::remove_dir(fixture.root.join("transport")).unwrap();
    directory(&fixture.root.join("transport"));

    assert!(approved.revalidate().is_err());
    // 新批准必须能正常建立：旧 FD 还攥在手里不构成对新能力的误判。
    let reopened = fixture.approve(&scope).unwrap();
    assert_eq!(reopened.revalidate(), Ok(()));
    // 旧能力不会因为有人重新批准就复活。
    assert!(approved.revalidate().is_err());
}

/// 镜像根同样按对象钉；删掉重建、换成另一个目录都必须让既有能力失效。
#[test]
fn capability_revalidation_fails_after_the_mirror_root_object_is_replaced() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();

    fs::remove_dir(fixture.root.join("mirror")).unwrap();
    directory(&fixture.root.join("mirror"));

    assert!(approved.revalidate().is_err());
}

/// 换成另一个**确实不同**的目录（先建后改名顶上），任何平台都必须识别。
#[test]
fn capability_revalidation_fails_when_the_root_path_is_renamed_onto_another_directory() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();

    let replacement = fixture.root.join("replacement");
    directory(&replacement);
    fs::remove_dir(fixture.root.join("transport")).unwrap();
    fs::rename(&replacement, fixture.root.join("transport")).unwrap();

    assert!(approved.revalidate().is_err());
}

#[test]
fn capability_revalidation_fails_after_the_mirror_root_loses_private_permissions() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();

    fs::set_permissions(
        fixture.root.join("mirror"),
        <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o750),
    )
    .unwrap();

    assert!(approved.revalidate().is_err());
}

/// 根被删掉后不重建，能力同样失效；这一条在改用 FD 之前也成立，留作回归。
#[test]
fn capability_revalidation_fails_after_the_private_root_is_removed_outright() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();

    fs::remove_dir(fixture.root.join("transport")).unwrap();

    assert!(approved.revalidate().is_err());
}

/// 复验所依赖的平台事实：删掉一个还被 FD 持有的目录之后，至少有一条信号仍能识别它。
///
/// Linux：持有 FD 的 `fstat` 链接数归零（实测 overlayfs on ext4 20/20），
/// 且旧 inode 没释放前那个号发不出去，同名新目录拿到的是另一个号。
/// macOS/APFS：链接数不归零，但 inode 号从不复用，靠第二条覆盖。
///
/// 两条都不成立的平台，`ApprovedTransport::revalidate` 就挡不住 inode 复用，
/// 必须显式归类为不支持而不是默默放行——所以这条断言写成「或」，一旦全失效就变红。
#[test]
fn a_removed_directory_stays_recognisable_through_the_descriptor_that_pinned_it() {
    let fixture = Fixture::new();
    let path = fixture.root.join("probe");
    directory(&path);
    let held = fs::File::open(&path).unwrap();
    let pinned = held.metadata().unwrap();
    let (pinned_dev, pinned_ino) = (pinned.dev(), pinned.ino());

    fs::remove_dir(&path).unwrap();
    let removed = held.metadata().unwrap();
    directory(&path);
    let fresh = fs::symlink_metadata(&path).unwrap();

    let link_count_cleared = removed.nlink() == 0;
    let inode_not_recycled = (fresh.dev(), fresh.ino()) != (pinned_dev, pinned_ino);
    // 分支自证：这条断言写成「或」，任一条信号成立就通过；日志不写清本平台
    // 实际靠的是哪一条，另一条是否早已失效就无从发现。
    eprintln!(
        "已删目录可识别性·分支自证：nlink 归零={link_count_cleared}（nlink={}）、\
         inode 未复用={inode_not_recycled}",
        removed.nlink()
    );
    assert!(
        link_count_cleared || inode_not_recycled,
        "本平台两条信号同时失效：nlink={} 复用={:?}",
        removed.nlink(),
        !inode_not_recycled
    );
}

/// 无关目录被替换不影响已批准的根；复验不能有假阳性。
#[test]
fn replacing_an_unrelated_directory_does_not_invalidate_the_capability() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();

    let unrelated: &Path = &fixture.root.join("canonical");
    fs::remove_dir(unrelated).unwrap();
    directory(unrelated);

    assert_eq!(approved.revalidate(), Ok(()));
    assert_eq!(approved.revalidate(), Ok(()));
}
