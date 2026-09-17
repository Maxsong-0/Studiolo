use super::fixture::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
use studiolo_host_config::SyncScopeError as Error;

#[test]
fn scope_hash_covers_domain_length_and_actual_entire_private_config_bytes() {
    let f = Fixture::new();
    let bytes = fs::read(&f.path).unwrap();
    let scope = f.scope();
    let mut hash = Sha256::new();
    hash.update(b"studiolo.sync-scope.private-config.v1\0");
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(&bytes);
    let expected: [u8; 32] = hash.finalize().into();
    assert_eq!(scope.binding().scope_hash, expected);
    assert_eq!(scope.binding().vault_uuid, f.vault);
    assert_eq!(scope.binding().link_id, [0x11; 16]);
    assert_eq!(scope.binding().remote_vault, [0x22; 16]);
    assert_eq!(fs::read(&f.path).unwrap(), bytes);
    for name in ["auth", "audit"] {
        assert_eq!(fs::read_dir(f.root.join(name)).unwrap().count(), 0);
    }
}

#[test]
fn same_length_configuration_change_invalidates_old_scope_even_outside_scope_fields() {
    let f = Fixture::new();
    let old = f.scope();
    let before = fs::read(&f.path).unwrap();
    let mut changed = f.value.clone();
    changed["seed_ref"] = json!("changed-seed-id");
    f.write_value(&changed);
    assert_eq!(fs::metadata(&f.path).unwrap().len(), before.len() as u64);
    assert!(old.revalidate().is_err());
    assert!(matches!(
        old.approve_union(old.binding(), &[], &[], &[]),
        Err(Error::ConfigurationChanged)
    ));
    let fresh = f.scope();
    assert_ne!(fresh.binding().scope_hash, old.binding().scope_hash);
    assert!(matches!(
        fresh.approve_union(old.binding(), &[], &[], &[]),
        Err(Error::BindingMismatch)
    ));
}

#[test]
fn formatting_changes_hash_and_new_config_object_does_not_refresh_old_pin() {
    let f = Fixture::new();
    let old = f.scope();
    let mut bytes = fs::read(&f.path).unwrap();
    bytes.push(b' ');
    f.bytes(&bytes);
    let fresh = f.scope();
    assert_ne!(old.binding().scope_hash, fresh.binding().scope_hash);
    assert!(old.revalidate().is_err());
    assert!(fresh.revalidate().is_ok());
}

#[test]
fn config_inode_symlink_hardlink_permissions_and_parent_replacement_fail_closed() {
    for attack in 0..5 {
        let f = Fixture::new();
        let scope = f.scope();
        let bytes = fs::read(&f.path).unwrap();
        let other = f.root.join("config/other.json");
        match attack {
            0 => {
                fs::write(&other, &bytes).unwrap();
                fs::set_permissions(&other, fs::Permissions::from_mode(0o600)).unwrap();
                fs::rename(&other, &f.path).unwrap();
            }
            1 => {
                fs::rename(&f.path, &other).unwrap();
                symlink(&other, &f.path).unwrap();
            }
            2 => fs::hard_link(&f.path, &other).unwrap(),
            3 => fs::set_permissions(&f.path, fs::Permissions::from_mode(0o640)).unwrap(),
            _ => {
                fs::rename(f.root.join("config"), f.root.join("old-config")).unwrap();
                directory(&f.root.join("config"));
                f.bytes(&bytes);
            }
        }
        assert!(matches!(
            scope.approve_union(scope.binding(), &[], &[], &[]),
            Err(Error::ConfigurationChanged)
        ));
    }
}

#[test]
fn widening_scope_with_same_epoch_changes_binding_and_cannot_reuse_old_baseline() {
    let mut f = Fixture::new();
    let old = f.scope();
    f.value["sync_scope"]["whole_folders"]
        .as_array_mut()
        .unwrap()
        .push(folder("新增"));
    f.write();
    let fresh = f.scope();
    assert_eq!(fresh.binding().scope_epoch, old.binding().scope_epoch);
    assert_ne!(fresh.binding().scope_hash, old.binding().scope_hash);
    assert!(matches!(
        fresh.approve_union(old.binding(), &[], &[], &[]),
        Err(Error::BindingMismatch)
    ));
}
