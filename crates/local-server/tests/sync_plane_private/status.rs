//! 同步状态回执的编解码与重启起点（T-224）：合法的读得回，任何一处不对都读不出来；
//! 只有 `Idle`／`Settled` 算对上；首装从打开时刻起算，坏了记空。
use super::*;

fn receipt(last_aligned_at: Option<i64>, deletes_blocked: Option<u64>) -> StatusReceipt {
    StatusReceipt {
        schema: SCHEMA,
        written_at: 1_800_000_000,
        last_aligned_at,
        deletes_blocked,
    }
}

#[test]
fn 写出的字节固定四个字段空值也写出来() {
    let bytes = encode(&receipt(None, None)).unwrap();
    assert_eq!(
        bytes,
        br#"{"schema":1,"written_at":1800000000,"last_aligned_at":null,"deletes_blocked":null}"#
    );
    let full = receipt(Some(1_799_990_000), Some(1309));
    assert_eq!(decode(&encode(&full).unwrap()), Some(full));
    assert_eq!(decode(&bytes), Some(receipt(None, None)));
}

#[test]
fn 字段缺多重复类型版本不对都读不出来() {
    for bad in [
        // 缺字段：可空字段也不能省。
        br#"{"schema":1,"written_at":10,"last_aligned_at":5}"#.as_slice(),
        br#"{"schema":1,"written_at":10,"deletes_blocked":0}"#,
        // 多字段。
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":0,"extra":1}"#,
        // 重复键。
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"last_aligned_at":6,"deletes_blocked":0}"#,
        // 版本、类型。
        br#"{"schema":2,"written_at":10,"last_aligned_at":5,"deletes_blocked":0}"#,
        br#"{"schema":1,"written_at":"10","last_aligned_at":5,"deletes_blocked":0}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":5.5,"deletes_blocked":0}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":-1}"#,
        br#"{"schema":true,"written_at":10,"last_aligned_at":5,"deletes_blocked":0}"#,
        // 时刻：负数、对上晚于写出。
        br#"{"schema":1,"written_at":-1,"last_aligned_at":null,"deletes_blocked":0}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":-1,"deletes_blocked":0}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":11,"deletes_blocked":0}"#,
        // 不是对象、空、截断。
        br#"[1,10,5,0]"#,
        b"",
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":0"#,
    ] {
        assert_eq!(decode(bad), None, "{}", String::from_utf8_lossy(bad));
    }
    // 边界：对上时刻等于写出时刻、0 都合法。
    assert!(
        decode(br#"{"schema":1,"written_at":10,"last_aligned_at":10,"deletes_blocked":0}"#)
            .is_some()
    );
    assert!(
        decode(br#"{"schema":1,"written_at":0,"last_aligned_at":0,"deletes_blocked":null}"#)
            .is_some()
    );
}

#[test]
fn 超过上限的回执写不出也读不进() {
    let mut padded = encode(&receipt(Some(1), Some(0))).unwrap();
    padded.splice(1..1, std::iter::repeat_n(b' ', MAX_BYTES));
    assert!(padded.len() > MAX_BYTES);
    assert_eq!(decode(&padded), None);
}

#[test]
fn 只有空闲和一批闭合算对上() {
    assert!(aligned(&Ok(RoundOutcome::Idle)));
    assert!(aligned(&Ok(RoundOutcome::Settled {
        sequence: 3,
        baseline_advanced: false,
    })));
    for outcome in [
        Ok(RoundOutcome::Skipped),
        Ok(RoundOutcome::Recovered),
        Ok(RoundOutcome::Unknown { sequence: 11 }),
        Ok(RoundOutcome::Blocked { sequence: 11 }),
        Ok(RoundOutcome::DeletesBlocked { count: 1309 }),
        Err(PlaneError::Fused),
        Err(PlaneError::Unknown),
        Err(PlaneError::Storage),
        Err(PlaneError::Transport),
        Err(PlaneError::Backoff),
        Err(PlaneError::Stopping),
    ] {
        assert!(!aligned(&outcome), "{outcome:?}");
    }
}

#[test]
fn 重启起点首装从打开时刻起算坏了记空() {
    let now = 1_800_000_000;
    assert_eq!(
        resume(&Previous::Absent, now),
        Resumed {
            last_aligned_at: Some(now),
            deletes_blocked: None,
        }
    );
    assert_eq!(
        resume(&Previous::Corrupt, now),
        Resumed {
            last_aligned_at: None,
            deletes_blocked: None,
        }
    );
    // 合法回执：对上时刻和被挡数都沿用；上一个进程已记空的保持空。
    let earlier = receipt(Some(now - 7 * 3600), Some(1309));
    assert_eq!(
        resume(&Previous::Valid(earlier), now),
        Resumed {
            last_aligned_at: Some(now - 7 * 3600),
            deletes_blocked: Some(1309),
        }
    );
    assert_eq!(
        resume(&Previous::Valid(receipt(None, Some(0))), now).last_aligned_at,
        None
    );
    // 时钟回拨：记着的对上时刻在现在之后，说不清过了多久，记空。
    assert_eq!(
        resume(&Previous::Valid(receipt(Some(now + 1), None)), now).last_aligned_at,
        None
    );
    assert_eq!(
        resume(&Previous::Valid(receipt(Some(now), None)), now).last_aligned_at,
        Some(now)
    );
}
