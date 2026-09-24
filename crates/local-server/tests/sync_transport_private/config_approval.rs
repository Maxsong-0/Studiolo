//! 按审批核对模式、排除项与五类附件：两向不一致都拒绝，同集合不同顺序视为一致。

use super::*;
use serde_json::json;

const ALL_FIVE: [FileType; 5] = [
    FileType::Image,
    FileType::Audio,
    FileType::Video,
    FileType::Pdf,
    FileType::Unsupported,
];

fn excluded(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn verify_with(
    config: &serde_json::Value,
    expected: &Expected<'_>,
) -> Result<LocalConfigMatch, ConfigError> {
    verify(&serde_json::to_vec(config).unwrap(), expected)
}

#[test]
fn sync_mode_must_match_the_approved_mode_in_both_directions() {
    // 审批 bidirectional、headless 报 mirror-remote。
    let mut reported = value();
    reported["syncMode"] = json!("mirror-remote");
    assert_eq!(check(&reported), Err(ConfigError::Mismatch));
    // 审批 mirror-remote、headless 报 bidirectional。
    let mut approval = expected();
    approval.sync_mode = "mirror-remote";
    assert_eq!(verify_with(&value(), &approval), Err(ConfigError::Mismatch));
    // 两侧同为 mirror-remote 才一致。
    assert_eq!(verify_with(&reported, &approval), Ok(LocalConfigMatch));
    // 审批值本身不在两值之内是调用方错误，不是 headless 的不一致。
    approval.sync_mode = "pull-only";
    assert_eq!(
        verify_with(&reported, &approval),
        Err(ConfigError::InvalidExpected)
    );
}

#[test]
fn excluded_folders_are_compared_as_a_set_against_the_sorted_approval() {
    let approved = excluded(&["School", "日记"]);
    let mut approval = expected();
    approval.excluded_folders = &approved;
    let mut config = value();
    // 同集合、不同顺序：一致。
    config["excludedFolders"] = json!(["日记", "School"]);
    assert_eq!(verify_with(&config, &approval), Ok(LocalConfigMatch));
    // 多一项。
    config["excludedFolders"] = json!(["School", "日记", "Archive"]);
    assert_eq!(verify_with(&config, &approval), Err(ConfigError::Mismatch));
    // 少一项。
    config["excludedFolders"] = json!(["School"]);
    assert_eq!(verify_with(&config, &approval), Err(ConfigError::Mismatch));
    // 空报告对非空审批同样不一致。
    config["excludedFolders"] = json!([]);
    assert_eq!(verify_with(&config, &approval), Err(ConfigError::Mismatch));
    // 超过 64 项是格式错误，先于一致性判定。
    let many: Vec<String> = (0..65).map(|index| format!("f{index}")).collect();
    config["excludedFolders"] = json!(many);
    assert_eq!(verify_with(&config, &approval), Err(ConfigError::Malformed));
}

#[test]
fn unsorted_duplicate_or_oversized_expected_exclusions_are_invalid_expected() {
    let unsorted = excluded(&["日记", "School"]);
    let mut approval = expected();
    approval.excluded_folders = &unsorted;
    assert_eq!(
        verify_with(&value(), &approval),
        Err(ConfigError::InvalidExpected)
    );
    let duplicate = excluded(&["School", "School"]);
    approval.excluded_folders = &duplicate;
    assert_eq!(
        verify_with(&value(), &approval),
        Err(ConfigError::InvalidExpected)
    );
    let many: Vec<String> = (0..65).map(|index| format!("f{index:03}")).collect();
    approval.excluded_folders = &many;
    assert_eq!(
        verify_with(&value(), &approval),
        Err(ConfigError::InvalidExpected)
    );
}

#[test]
fn the_five_attachment_kinds_must_all_be_present_when_all_five_are_approved() {
    let mut approval = expected();
    approval.file_types = &ALL_FIVE;
    let mut config = value();
    config["fileTypes"] = json!(["image", "audio", "video", "pdf", "unsupported"]);
    assert_eq!(verify_with(&config, &approval), Ok(LocalConfigMatch));
    // 缺 unsupported：txt／json 类附件就进不了镜像。
    config["fileTypes"] = json!(["image", "audio", "video", "pdf"]);
    assert_eq!(verify_with(&config, &approval), Err(ConfigError::Mismatch));
    // 顺序无关。
    config["fileTypes"] = json!(["unsupported", "pdf", "video", "audio", "image"]);
    assert_eq!(verify_with(&config, &approval), Ok(LocalConfigMatch));
    // 审批只批四类、headless 多开 unsupported 同样不一致。
    let four = [
        FileType::Image,
        FileType::Audio,
        FileType::Video,
        FileType::Pdf,
    ];
    approval.file_types = &four;
    config["fileTypes"] = json!(["image", "audio", "video", "pdf", "unsupported"]);
    assert_eq!(verify_with(&config, &approval), Err(ConfigError::Mismatch));
}
