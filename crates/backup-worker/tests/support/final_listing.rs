use super::*;
use crate::seal_fixture;
use serde_json::json;

#[test]
fn final_tree_has_exactly_one_fixed_regular_file_and_one_matching_header() {
    let marker = Marker::for_job(&seal_fixture::job("final-listing", u64::MAX)).unwrap();
    assert!(marker.tag.as_str().len() <= 64);
    let id = SnapshotRef::parse("cd".repeat(32)).unwrap();
    let records = [
        json!({"message_type":"snapshot","id":id.as_str(),"tree":"ef".repeat(32),"paths":[marker.path],"tags":[marker.tag.as_str()]}),
        json!({"message_type":"node","name":marker.path.file_name().unwrap().to_str().unwrap(),"path":marker.path,"type":"file","mode":420,"size":123}),
    ];
    let encoded = |rows: &[serde_json::Value]| {
        rows.iter()
            .map(|row| serde_json::to_string(row).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    };
    verify(encoded(&records).as_bytes(), &marker, &id, 123).unwrap();
    for bad in 0..10 {
        let mut rows = records.to_vec();
        match bad {
            0 => rows[1]["size"] = json!(124),
            1 => rows[1]["type"] = json!("symlink"),
            2 => rows[1]["mode"] = json!(493),
            3 => rows[1]["name"] = json!("other"),
            4 => rows[1]["path"] = json!("/other"),
            5 => rows[0]["tags"] = json!(["another-job.final"]),
            6 => {
                rows.remove(1);
            }
            7 => rows.push(rows[1].clone()),
            8 => rows.push(rows[0].clone()),
            _ => rows.reverse(),
        }
        assert!(
            verify(encoded(&rows).as_bytes(), &marker, &id, 123).is_err(),
            "bad={bad}"
        );
    }
}
