//! 磁盘与 Sync 配额的行为／失败／恢复三类契约。
//!
//! 重点：等号可过、溢出拒绝、读数缺失不得放行、超限必须阻断写入而不是只告警。

use studiolo_observability::quota::{
    Admission, BlockReason, DiskQuota, DiskReading, QuotaError, QuotaVerdict, SyncQuota, SyncUsage,
};

fn disk() -> DiskQuota {
    DiskQuota::new(1_000, 5_000).expect("固定测试水位必须合法")
}

fn sync() -> SyncQuota {
    SyncQuota::new(100, 10_000, 80, 8_000).expect("固定测试上限必须合法")
}

#[test]
fn 磁盘剩余在预警水位之上时判定为正常() {
    assert_eq!(
        disk().evaluate(Some(DiskReading { free_bytes: 9_000 })),
        QuotaVerdict::Normal
    );
}

#[test]
fn 磁盘剩余跌破预警水位但仍高于下限时判定为预警() {
    assert_eq!(
        disk().evaluate(Some(DiskReading { free_bytes: 2_000 })),
        QuotaVerdict::Warning
    );
}

#[test]
fn 磁盘剩余跌破下限时判定为超限() {
    assert_eq!(
        disk().evaluate(Some(DiskReading { free_bytes: 999 })),
        QuotaVerdict::Exceeded
    );
}

#[test]
fn 磁盘读数缺失时判定为未知而不是正常() {
    assert_eq!(disk().evaluate(None), QuotaVerdict::Unknown);
}

#[test]
fn 写入后剩余空间刚好等于下限时放行等号() {
    let admission = disk()
        .admit(Some(DiskReading { free_bytes: 3_000 }), 2_000)
        .expect("等号不应触发溢出");
    assert_eq!(admission, Admission::Allowed);
}

#[test]
fn 写入后剩余空间比下限少一个字节时阻断写入() {
    let admission = disk()
        .admit(Some(DiskReading { free_bytes: 3_000 }), 2_001)
        .expect("普通减法不应溢出");
    assert_eq!(
        admission,
        Admission::Blocked(BlockReason::DiskFreeBelowMinimum)
    );
}

#[test]
fn 写入字节数超过当前剩余时按整数越界拒绝而不是回绕放行() {
    assert_eq!(
        disk().admit(Some(DiskReading { free_bytes: 10 }), u64::MAX),
        Err(QuotaError::ArithmeticOverflow)
    );
}

#[test]
fn 磁盘读数缺失时写入准入一律阻断而不是默认放行() {
    let admission = disk().admit(None, 1).expect("缺读数不走算术路径");
    assert_eq!(
        admission,
        Admission::Blocked(BlockReason::DiskFreeBelowMinimum)
    );
}

#[test]
fn 预警水位比阻断下限更紧的配置直接判为不自洽() {
    assert_eq!(DiskQuota::new(5_000, 1_000), Err(QuotaError::InvalidLimits));
    assert_eq!(DiskQuota::new(0, 1_000), Err(QuotaError::InvalidLimits));
}

#[test]
fn sync占用刚好等于两条上限时仍然放行等号() {
    let admission = sync()
        .admit(
            Some(SyncUsage {
                objects: 99,
                bytes: 9_999,
            }),
            1,
            1,
        )
        .expect("等号不应触发溢出");
    assert_eq!(admission, Admission::Allowed);
}

#[test]
fn sync对象数超过上限时按对象维度阻断() {
    let admission = sync()
        .admit(
            Some(SyncUsage {
                objects: 100,
                bytes: 0,
            }),
            1,
            0,
        )
        .expect("普通加法不应溢出");
    assert_eq!(admission, Admission::Blocked(BlockReason::SyncObjectLimit));
}

#[test]
fn sync字节数超过上限时按字节维度阻断() {
    let admission = sync()
        .admit(
            Some(SyncUsage {
                objects: 0,
                bytes: 10_000,
            }),
            0,
            1,
        )
        .expect("普通加法不应溢出");
    assert_eq!(admission, Admission::Blocked(BlockReason::SyncByteLimit));
}

#[test]
fn sync增量导致加法越界时一律拒绝而不是回绕成小数() {
    assert_eq!(
        sync().admit(
            Some(SyncUsage {
                objects: 1,
                bytes: 1
            }),
            u64::MAX,
            0
        ),
        Err(QuotaError::ArithmeticOverflow)
    );
    assert_eq!(
        sync().admit(
            Some(SyncUsage {
                objects: 1,
                bytes: 1
            }),
            0,
            u64::MAX
        ),
        Err(QuotaError::ArithmeticOverflow)
    );
}

#[test]
fn sync占用证据缺失时写入准入一律阻断() {
    let admission = sync().admit(None, 0, 0).expect("缺证据不走算术路径");
    assert_eq!(admission, Admission::Blocked(BlockReason::SyncObjectLimit));
}

#[test]
fn sync占用越过预警水位但未越上限时判定为预警() {
    assert_eq!(
        sync().evaluate(Some(SyncUsage {
            objects: 80,
            bytes: 0
        })),
        QuotaVerdict::Warning
    );
}

#[test]
fn 配额从超限回落到预算内之后重新放行写入() {
    let quota = sync();
    let over = quota
        .admit(
            Some(SyncUsage {
                objects: 100,
                bytes: 0,
            }),
            1,
            0,
        )
        .expect("普通加法不应溢出");
    assert!(!over.allowed());

    let recovered = quota
        .admit(
            Some(SyncUsage {
                objects: 10,
                bytes: 10,
            }),
            1,
            1,
        )
        .expect("普通加法不应溢出");

    assert!(recovered.allowed());
    assert_eq!(
        quota.evaluate(Some(SyncUsage {
            objects: 10,
            bytes: 10
        })),
        QuotaVerdict::Normal
    );
}

#[test]
fn 配额结论的默认值是未知而不是正常() {
    assert_eq!(QuotaVerdict::default(), QuotaVerdict::Unknown);
}
