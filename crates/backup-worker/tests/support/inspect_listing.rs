use super::*;
use crate::seal_fixture::{Fixture, job};
use std::{fs, os::unix::fs::MetadataExt};
use studiolo_backup::worker::restic::JobTag;

#[test]
fn listing_requires_exact_directories_files_identity_and_single_header() {
    let f = Fixture::new();
    let seal = f.seal("listing");
    let root = &seal.ticket().transfer_source.root;
    let key = job("listing", 1);
    let snapshot = SnapshotRef::parse("ab".repeat(32)).unwrap();
    let stat = fs::metadata(root).unwrap();
    let mut rows = vec![
        serde_json::json!({"message_type": "snapshot", "id":snapshot.as_str(), "tree":"cd".repeat(32), "paths":[root], "tags":[JobTag::for_job(&key).unwrap().as_str()]}),
    ];
    let mut dirs = root
        .ancestors()
        .filter(|p| *p != Path::new("/"))
        .map(Path::to_owned)
        .collect::<Vec<_>>();
    dirs.extend(seal.inventory().directories().map(|p| root.join(p)));
    for path in dirs {
        rows.push(serde_json::json!({"message_type":"node", "name": path.file_name().unwrap().to_str().unwrap(), "path":path, "type":"dir", "mode":(1_u32<<31)|0o550, "uid":stat.uid(), "gid":stat.gid()}));
    }
    for file in seal.inventory().files() {
        let path = root.join(file.path().as_str());
        rows.push(serde_json::json!({"message_type":"node", "name":path.file_name().unwrap().to_str().unwrap(), "path":path, "type":"file", "mode":0o440, "uid":stat.uid(), "gid":stat.gid(), "size":file.bytes()}));
    }
    let check = |rows: &[serde_json::Value]| {
        let bytes = rows
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        verify(
            bytes.as_bytes(),
            Expected {
                key: &key,
                snapshot: &snapshot,
                root,
                inventory: seal.inventory(),
                producer: stat.uid(),
                group: stat.gid(),
            },
        )
    };
    check(&rows).unwrap();
    for change in 0..9 {
        let mut changed = rows.clone();
        let last = changed.len() - 1;
        match change {
            0 => {
                changed.pop();
            }
            1 => {
                changed.push(changed[last].clone());
            }
            2 => {
                changed[last]["path"] = serde_json::json!(root.join("unexpected"));
                changed[last]["name"] = serde_json::json!("unexpected");
            }
            3 => changed[last]["size"] = serde_json::json!(0),
            4 => changed[last]["type"] = serde_json::json!("symlink"),
            5 => changed[last]["uid"] = serde_json::json!(stat.uid() + 1),
            6 => changed[last]["mode"] = serde_json::json!(0o640),
            7 => {
                changed.push(changed[0].clone());
            }
            _ => {
                changed.swap(0, 1);
            }
        }
        assert!(check(&changed).is_err(), "change={change}");
    }
}
