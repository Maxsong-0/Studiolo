use super::*;
use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

#[test]
fn config_input_requires_one_readonly_regular_bounded_original_file() {
    for mode in 0..8 {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        fs::write(&path, b"fictional config").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut uid = fs::metadata(&path).unwrap().uid();
        let mut deadline = Instant::now() + Duration::from_secs(10);
        match mode {
            0 => {}
            1 => fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap(),
            2 => fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap(),
            3 => fs::hard_link(&path, temp.path().join("alias")).unwrap(),
            4 => fs::write(&path, [b'x'; 16 * 1024 + 1]).unwrap(),
            5 => fs::write(&path, []).unwrap(),
            6 => uid += 1,
            _ => deadline = Instant::now() - Duration::from_secs(1),
        }
        let result = read_checked(File::open(&path).unwrap(), uid, deadline);
        assert_eq!(result.is_ok(), mode <= 1, "mode={mode}");
        if let Ok((bytes, _)) = result {
            assert_eq!(bytes, b"fictional config");
        }
    }
}
