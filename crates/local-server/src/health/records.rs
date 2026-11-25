//! 「写者记录」一节（BUG-051、T-235）：写者那几处只增不减、满了就拒绝新写入的记录，离上限还有多远。
//!
//! 哪一处最满就说哪一处；到上限的八成降级，没读到是未知，不显示绿。正文只有条数、百分比和固定中文，
//! 且不出现斜杠（脱敏边界不许）。
use studiolo_observability::health::{HealthSection, HealthState};
use studiolo_vault_broker::{RecordUsage, RecordUsageItem};

/// 本节的稳定机器码。
pub const RECORD_USAGE_CODE: &str = "writer-records";
const TITLE: &str = "写者记录";

/// 写者记录的用量证据。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordUsageEvidence {
    /// 没有装配写者（只读服务）。
    Disabled,
    /// 装配了但这一次没读到。
    #[default]
    Unreadable,
    /// 最近一次量到的。
    Read(RecordUsage),
}

/// 用量的来源；实现只读内存里最近一次量到的，不碰磁盘、不等写者锁。
pub trait RecordUsageProbe: Send + Sync {
    /// 最近一次量到的用量；还没量过、上一次没量成都是 `None`。
    fn record_usage(&self) -> Option<RecordUsage>;
}

/// 一处记录：名字、满了之后会怎样。
struct Place {
    name: &'static str,
    item: RecordUsageItem,
    full: &'static str,
}

/// 固定顺序；并列最满时说排在前面的。
fn places(usage: &RecordUsage) -> [Place; 8] {
    const SAVES: &str = "新的保存会被拒";
    const REPLAYS: &str = "备份封存、新建笔记、改名和删除会被拒";
    const FILES: &str = "改名和删除会被拒";
    let place = |name, item, full| Place { name, item, full };
    [
        place("正文记录条数", usage.descriptors, SAVES),
        place("正文记录字节", usage.descriptor_bytes, SAVES),
        place("幂等重放条数", usage.replays, REPLAYS),
        place("幂等重放字节", usage.replay_bytes, REPLAYS),
        place("文件操作条数", usage.file_operations, FILES),
        place("文件操作字节", usage.file_operation_bytes, FILES),
        place("墓碑名额", usage.tombstones, FILES),
        place("打包段数", usage.segments, "旧记录打不了包"),
    ]
}

/// 已用的百分比，向下取整；没有上限（没启用）的那一处不算。
fn percent(item: RecordUsageItem) -> Option<u128> {
    (item.limit > 0).then(|| u128::from(item.used) * 100 / u128::from(item.limit))
}

/// 本节结论；正文越出脱敏边界只可能是实现缺陷，退成未知。
pub(super) fn records_section(evidence: &RecordUsageEvidence) -> HealthSection {
    let (state, detail) = match evidence {
        RecordUsageEvidence::Disabled => (
            HealthState::Unknown,
            "没有装配写者，看不到写者记录的用量".to_owned(),
        ),
        RecordUsageEvidence::Unreadable => (
            HealthState::Unknown,
            "这一次没读到写者记录的用量，状态未知".to_owned(),
        ),
        RecordUsageEvidence::Read(usage) => judge(usage),
    };
    HealthSection::new(RECORD_USAGE_CODE, TITLE, state, detail)
        .or_else(|_| {
            HealthSection::new(
                RECORD_USAGE_CODE,
                TITLE,
                HealthState::Unknown,
                "写者记录的结论没能写成脱敏正文，按未知处理",
            )
        })
        .expect("固定健康文案必须满足脱敏契约")
}

fn judge(usage: &RecordUsage) -> (HealthState, String) {
    let places = places(usage);
    let said = |place: &Place| {
        format!(
            "{} {}／{}，已到上限的 {}%",
            place.name,
            place.item.used,
            place.item.limit,
            percent(place.item).unwrap_or(100)
        )
    };
    let alarmed: Vec<&Place> = places
        .iter()
        .filter(|place| place.item.nearly_full())
        .collect();
    let packed = format!("已打包 {} 条", usage.packs);
    // 最满的那一处；并列取排在前面的。
    let fullest = |among: &[&Place]| {
        among
            .iter()
            .copied()
            .rev()
            .max_by_key(|place| percent(place.item).unwrap_or(0))
            .map(|place| (place.name, said(place), place.full))
    };
    if let Some((name, sentence, full)) = fullest(&alarmed) {
        let others: Vec<&str> = alarmed
            .iter()
            .map(|place| place.name)
            .filter(|other| *other != name)
            .collect();
        let also = if others.is_empty() {
            String::new()
        } else {
            format!("；另有：{}", others.join("、"))
        };
        return (
            HealthState::Degraded,
            format!("{sentence}：满了之后{full}{also}；{packed}"),
        );
    }
    let all: Vec<&Place> = places.iter().collect();
    let sentence = fullest(&all).map_or_else(String::new, |(_, sentence, _)| sentence);
    (
        HealthState::Normal,
        format!("最满的一处：{sentence}；{packed}"),
    )
}
