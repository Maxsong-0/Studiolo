//! 非秘密配置与公开目录必须仍按真实对象身份隔离，不能穿过链接或修权限。

use super::fixture::{Fixture, mode};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

#[test]
fn private_modes_are_rejected_without_repair() {
    for target in 0..4 {
        let fixture = Fixture::new();
        let path = match target {
            0 => &fixture.config,
            1 => fixture.config.parent().unwrap(),
            2 => &fixture.auth,
            _ => &fixture.audit,
        };
        let wrong = if target == 0 { 0o644 } else { 0o755 };
        mode(path, wrong);
        fixture.reject(fixture.args());
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o7777,
            wrong
        );
        fixture.empty_stores();
    }
}

#[test]
fn public_roots_cannot_be_relative_or_symbolic_links() {
    for (flag, relative) in [("--vault", "vault"), ("--assets", "web")] {
        let fixture = Fixture::new();
        let mut args = fixture.args();
        let index = args.iter().position(|value| value == flag).unwrap() + 1;
        args[index] = relative.into();
        fixture.reject(args);
        let link = fixture.root.join("public-link");
        symlink(
            if flag == "--vault" {
                &fixture.vault
            } else {
                &fixture.web
            },
            &link,
        )
        .unwrap();
        let mut args = fixture.args();
        args[index] = link.to_str().unwrap().into();
        fixture.reject(args);
        fixture.empty_stores();
    }
}

#[test]
fn private_roots_cannot_overlap_public_roots_in_either_direction() {
    for (flag, nested) in [
        ("--vault", false),
        ("--assets", false),
        ("--vault", true),
        ("--assets", true),
    ] {
        let fixture = Fixture::new();
        let mut args = fixture.args();
        let index = args.iter().position(|value| value == flag).unwrap() + 1;
        args[index] = (if nested { &fixture.root } else { &fixture.auth })
            .to_str()
            .unwrap()
            .into();
        fixture.reject(args);
        fixture.empty_stores();
    }
    let fixture = Fixture::new();
    let nested = fixture.vault.join("private-auth");
    fs::create_dir(&nested).unwrap();
    mode(&nested, 0o700);
    let mut value = fixture.value();
    value["auth_directory"] = json!(nested);
    fixture.write_config(&value);
    fixture.reject(fixture.args());
    assert_eq!(fs::read_dir(nested).unwrap().count(), 0);
}

#[test]
fn configuration_file_must_not_be_served_from_public_root() {
    for public in ["vault", "web"] {
        let fixture = Fixture::new();
        let exposed = fixture.root.join(public).join("owner.json");
        fs::copy(&fixture.config, &exposed).unwrap();
        mode(&exposed, 0o600);
        let mut args = fixture.args();
        let index = args
            .iter()
            .position(|value| value == "--session-config")
            .unwrap()
            + 1;
        args[index] = exposed.to_str().unwrap().into();
        fixture.reject(args);
        fixture.empty_stores();
    }
}

#[test]
fn configuration_links_and_two_store_aliases_are_rejected() {
    let fixture = Fixture::new();
    let link = fixture.config.parent().unwrap().join("linked.json");
    symlink(&fixture.config, &link).unwrap();
    let mut args = fixture.args();
    let index = args
        .iter()
        .position(|value| value == "--session-config")
        .unwrap()
        + 1;
    args[index] = link.to_str().unwrap().into();
    fixture.reject(args);
    let mut value = fixture.value();
    value["audit_directory"] = json!(fixture.auth);
    fixture.write_config(&value);
    fixture.reject(fixture.args());
    fixture.empty_stores();
}
