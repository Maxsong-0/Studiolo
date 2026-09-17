use super::fixture::{Fixture, directory};
use serde_json::json;
use std::{fs, os::unix::fs::symlink};

#[test]
fn repository_cannot_equal_or_be_inside_any_real_protected_root() {
    for name in [
        "config", "auth", "audit", "vault", "assets", "writer", "ingress",
    ] {
        for nested in [false, true] {
            let mut f = Fixture::new();
            let mut target = f.root.join(name);
            if nested {
                target = target.join("repository-child");
                directory(&target);
            }
            f.document["project_repository"]["directory"] = json!(target);
            f.write();
            assert!(f.approve().is_err(), "{name} nested={nested}");
        }
    }
}

#[test]
fn protected_roots_cannot_be_children_of_the_repository() {
    for name in [
        "config", "auth", "audit", "vault", "assets", "writer", "ingress",
    ] {
        let mut f = Fixture::new();
        let child = f.repository.join(name);
        directory(&child);
        match name {
            "config" => f.config = child.join("owner.json"),
            "auth" => f.document["auth_directory"] = json!(child),
            "audit" => f.document["audit_directory"] = json!(child),
            "vault" => f.vault = child,
            "assets" => f.assets = child,
            "writer" => f.writer = child,
            "ingress" => f.ingress = child,
            _ => unreachable!(),
        }
        f.write();
        assert!(f.approve().is_err(), "{name}");
    }
}

#[test]
fn root_and_ancestor_links_never_become_repository_approval() {
    for ancestor in [false, true] {
        let mut f = Fixture::new();
        let alias = f.root.join("alias");
        if ancestor {
            symlink(&f.root, &alias).unwrap();
            f.document["project_repository"]["directory"] = json!(alias.join("repository"));
        } else {
            symlink(&f.repository, &alias).unwrap();
            f.document["project_repository"]["directory"] = json!(alias);
        }
        f.write();
        assert!(f.approve().is_err());
    }
}

#[test]
fn case_aliases_use_filesystem_identity_and_case_sensitive_names_remain_distinct() {
    let mut f = Fixture::new();
    let alias = f.root.join("AUTH");
    if fs::symlink_metadata(&alias).is_ok() {
        // macOS 默认卷通常不区分大小写；不能凭不同字符串批准 Auth 别名。
        f.document["project_repository"]["directory"] = json!(alias);
        f.write();
        assert!(f.approve().is_err());
    } else {
        // Linux 常见区分大小写卷上，它是独立目录；先创建才能作为实际根批准。
        directory(&alias);
        f.document["project_repository"]["directory"] = json!(alias);
        f.write();
        assert!(f.approve().is_ok());
    }
}
