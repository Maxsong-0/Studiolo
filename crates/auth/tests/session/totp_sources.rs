//! 来源容量是独立存储护栏；不能通过驱逐仍有效的来源窗口接纳新来源。

use super::*;
use rusqlite::params;

#[test]
fn full_source_table_preserves_live_limits_and_recovers_only_after_window_expiry() {
    let h = TotpHarness::enrolled(3600, 7200);
    let owner = identity();
    let issued = h.h.manager.issue(&owner).unwrap();
    assert_eq!(
        h.totp
            .step_up(&request(&owner, &issued, 1), wrong_code(NEXT))
            .unwrap_err(),
        SessionError::Rejected
    );
    let existing = text_rows(
        &h.h.manager,
        "SELECT bucket FROM totp_rates WHERE bucket LIKE 'ip:%'",
    );
    assert_eq!(existing.len(), 1);
    // 单主人账号限额不允许一窗口真实建立64来源；此处仅为表饱和夹具，不代替128轮公开调用。
    {
        let connection = h.h.manager.database.connection.lock().unwrap();
        for index in 0..63 {
            let bucket = format!("ip:{index:064x}");
            assert_ne!(bucket, existing[0]);
            connection
                .execute(
                    "INSERT INTO totp_rates VALUES(?1,?2,1)",
                    params![bucket, NEXT],
                )
                .unwrap();
        }
    }
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_rates"), 66);
    let before = text_rows(
        &h.h.manager,
        "SELECT bucket FROM totp_rates ORDER BY bucket",
    );
    assert_eq!(
        h.totp
            .step_up(&request(&owner, &issued, 200), code(NEXT))
            .unwrap_err(),
        SessionError::Capacity
    );
    assert_eq!(
        text_rows(
            &h.h.manager,
            "SELECT bucket FROM totp_rates ORDER BY bucket"
        ),
        before
    );
    assert_eq!(scalar(&h.h.manager, "SELECT count(*) FROM totp_grants"), 0);
    // 已有来源仍可在各自预算内使用，没有把容量错误扩大成全部账号锁死。
    h.totp
        .step_up(&request(&owner, &issued, 1), code(NEXT))
        .unwrap();
    h.h.clock.set(NEXT + 900_000);
    let (_, totp, _) = h.reopen();
    totp.step_up(&request(&owner, &issued, 200), code(NEXT + 900_000))
        .unwrap();
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT count(*) FROM totp_rates WHERE bucket LIKE 'ip:%'"
        ),
        1
    );
}
