use super::*;
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn sqlite_vm_deadline_interrupts_work_and_removes_the_hook_for_following_reads() {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let directory = temp.path().join("policy");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let database = Database::open(&directory, CreatePolicy::AllowNew).unwrap();
    let start = Instant::now();
    let result = database.until(start + Duration::from_millis(30), |connection| {
        connection.query_row("WITH RECURSIVE counter(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM counter WHERE n<100000000) SELECT sum(n) FROM counter", [], |row| row.get::<_, i64>(0)).map_err(crate::database::unavailable)
    });
    assert_eq!(result, Err(PolicyError::Busy));
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(
        database
            .read(|tx| tx
                .query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
                .map_err(crate::database::unavailable))
            .unwrap(),
        42
    );
}
