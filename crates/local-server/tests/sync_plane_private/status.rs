//! 同步状态回执的编解码与重启起点（T-224）：合法的读得回，任何一处不对都读不出来；
//! 只有 `Idle`／`Settled` 算对上；首装从打开时刻起算，坏了记空；被挡数和证明它的时刻成对。
use super::*;

fn receipt(last_aligned_at: Option<i64>, gate: Option<(u64, i64)>) -> StatusReceipt {
    StatusReceipt {
        schema: SCHEMA,
        written_at: 1_800_000_000,
        last_aligned_at,
        deletes_blocked: gate.map(|(blocked, _)| blocked),
        deletes_checked_at: gate.map(|(_, at)| at),
    }
}

/// 契约第 2 节的示例行：Rust 写出的字节、探针（`deploy/alerts/test_monitor.py`）读进的字节都钉在它上面，
/// 三处的字段名、顺序、形状改一处另两处就红。
fn contract_example() -> &'static [u8] {
    const CONTRACT: &str = include_str!("../../../../docs/contracts/sync-status-receipt.md");
    let line = CONTRACT
        .lines()
        .find_map(|line| line.strip_prefix("例：`"))
        .expect("契约第 2 节要有一行「例：`…`」");
    line.split('`').next().unwrap().as_bytes()
}

#[test]
fn 写出的字节固定五个字段空值也写出来() {
    let bytes = encode(&receipt(None, None)).unwrap();
    assert_eq!(
        bytes,
        br#"{"schema":1,"written_at":1800000000,"last_aligned_at":null,"deletes_blocked":null,"deletes_checked_at":null}"#
    );
    let full = receipt(Some(1_799_990_000), Some((1309, 1_799_999_700)));
    assert_eq!(decode(&encode(&full).unwrap()), Some(full));
    assert_eq!(decode(&bytes), Some(receipt(None, None)));
    let example = decode(contract_example()).expect("契约示例必须读得进");
    assert_eq!(encode(&example).unwrap(), contract_example());
}

#[test]
fn 字段缺多重复类型版本不对都读不出来() {
    for bad in [
        // 缺字段：可空字段也不能省。
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":0}"#.as_slice(),
        br#"{"schema":1,"written_at":10,"deletes_blocked":0,"deletes_checked_at":5}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_checked_at":5}"#,
        // 多字段。
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":0,"deletes_checked_at":5,"extra":1}"#,
        // 重复键。
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"last_aligned_at":6,"deletes_blocked":0,"deletes_checked_at":5}"#,
        // 版本、类型。
        br#"{"schema":2,"written_at":10,"last_aligned_at":5,"deletes_blocked":0,"deletes_checked_at":5}"#,
        br#"{"schema":1,"written_at":"10","last_aligned_at":5,"deletes_blocked":0,"deletes_checked_at":5}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":5.5,"deletes_blocked":0,"deletes_checked_at":5}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":-1,"deletes_checked_at":5}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":0,"deletes_checked_at":true}"#,
        br#"{"schema":true,"written_at":10,"last_aligned_at":5,"deletes_blocked":0,"deletes_checked_at":5}"#,
        // 时刻：负数、晚于写出。
        br#"{"schema":1,"written_at":-1,"last_aligned_at":null,"deletes_blocked":null,"deletes_checked_at":null}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":-1,"deletes_blocked":0,"deletes_checked_at":5}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":11,"deletes_blocked":0,"deletes_checked_at":5}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":0,"deletes_checked_at":-1}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":0,"deletes_checked_at":11}"#,
        // 被挡数和证明时刻不成对。
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":1309,"deletes_checked_at":null}"#,
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":null,"deletes_checked_at":5}"#,
        // 不是对象、空、截断。
        br#"[1,10,5,0,5]"#,
        b"",
        br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":0,"deletes_checked_at":5"#,
    ] {
        assert_eq!(decode(bad), None, "{}", String::from_utf8_lossy(bad));
    }
    // 边界：时刻等于写出时刻、0、成对的空都合法。
    assert!(
        decode(br#"{"schema":1,"written_at":10,"last_aligned_at":10,"deletes_blocked":0,"deletes_checked_at":10}"#)
            .is_some()
    );
    assert!(
        decode(br#"{"schema":1,"written_at":0,"last_aligned_at":0,"deletes_blocked":null,"deletes_checked_at":null}"#)
            .is_some()
    );
}

#[test]
fn 超过上限的回执写不出也读不进() {
    let compact = encode(&receipt(Some(1), Some((0, 1)))).unwrap();
    // 补空白到正好上限仍是合法 JSON、读得进；再多一个字节就拒：两侧都钉住，删掉或挪动上限检查都会红。
    let mut padded = compact.clone();
    padded.splice(1..1, std::iter::repeat_n(b' ', MAX_BYTES - compact.len()));
    assert_eq!(padded.len(), MAX_BYTES);
    assert!(decode(&padded).is_some());
    padded.insert(1, b' ');
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
            deletes_gate: None,
        }
    );
    assert_eq!(
        resume(&Previous::Corrupt, now),
        Resumed {
            last_aligned_at: None,
            deletes_gate: None,
        }
    );
    // 合法回执：对上时刻和被挡的那一对都原样沿用（证明时刻不换成现在）；上一个进程已记空的保持空。
    let earlier = receipt(Some(now - 7 * 3600), Some((1309, now - 60)));
    assert_eq!(
        resume(&Previous::Valid(earlier), now),
        Resumed {
            last_aligned_at: Some(now - 7 * 3600),
            deletes_gate: Some((1309, now - 60)),
        }
    );
    assert_eq!(
        resume(&Previous::Valid(receipt(None, Some((0, now)))), now).last_aligned_at,
        None
    );
    // 时钟回拨：记着的时刻在现在之后，说不清过了多久，记空；等于现在照常沿用。
    assert_eq!(
        resume(&Previous::Valid(receipt(Some(now + 1), None)), now).last_aligned_at,
        None
    );
    assert_eq!(
        resume(&Previous::Valid(receipt(Some(now), None)), now).last_aligned_at,
        Some(now)
    );
    let ahead = Some((1309, now + 1));
    assert_eq!(
        resume(&Previous::Valid(receipt(Some(now), ahead)), now).deletes_gate,
        None
    );
    let level = Some((1309, now));
    assert_eq!(
        resume(&Previous::Valid(receipt(Some(now), level)), now).deletes_gate,
        level
    );
}
