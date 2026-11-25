//! 「写者记录」一节（BUG-051）：到八成降级并说出是哪一处，差一条不降；没读到不显示绿。
use super::{
    HealthBody, RecordUsage, RecordUsageEvidence, RecordUsageItem, complete, roomy_records,
};

const FALLBACK: &str = "写者记录的结论没能写成脱敏正文，按未知处理";

fn section(evidence: RecordUsageEvidence) -> (String, String) {
    let mut snapshot = complete(10_000);
    snapshot.records = evidence;
    let body = HealthBody::from_snapshot(&snapshot);
    let found = body
        .sections
        .iter()
        .find(|section| section.code == "writer-records")
        .expect("必须包含写者记录一节");
    assert_eq!(found.title, "写者记录");
    assert_ne!(found.detail, FALLBACK, "正文必须过得了脱敏边界");
    assert_eq!(
        body.overall == "normal",
        found.state == "normal",
        "别的节都正常，整体跟着这一节走"
    );
    (found.state.to_owned(), found.detail.clone())
}

fn read(change: impl FnOnce(&mut RecordUsage)) -> (String, String) {
    let mut usage = roomy_records();
    change(&mut usage);
    section(RecordUsageEvidence::Read(usage))
}

#[test]
fn normal_names_the_fullest_place() {
    assert_eq!(
        read(|usage| usage.packs = 12),
        (
            "normal".to_owned(),
            "最满的一处：幂等重放条数 1635／4096，已到上限的 39%；已打包 12 条".to_owned()
        )
    );
}

/// 每一处单独到八成都报，差一条不报；满了也只是同一档，说清楚会拒什么。
#[test]
fn every_place_degrades_at_eighty_percent_and_not_one_below() {
    type Pick = fn(&mut RecordUsage) -> &mut RecordUsageItem;
    let places: [(Pick, &str, &str); 8] = [
        (
            |usage| &mut usage.descriptors,
            "正文记录条数",
            "新的保存会被拒",
        ),
        (
            |usage| &mut usage.descriptor_bytes,
            "正文记录字节",
            "新的保存会被拒",
        ),
        (
            |usage| &mut usage.replays,
            "幂等重放条数",
            "备份封存、新建笔记、改名和删除会被拒",
        ),
        (
            |usage| &mut usage.replay_bytes,
            "幂等重放字节",
            "备份封存、新建笔记、改名和删除会被拒",
        ),
        (
            |usage| &mut usage.file_operations,
            "文件操作条数",
            "改名和删除会被拒",
        ),
        (
            |usage| &mut usage.file_operation_bytes,
            "文件操作字节",
            "改名和删除会被拒",
        ),
        (
            |usage| &mut usage.tombstones,
            "墓碑名额",
            "改名和删除会被拒",
        ),
        (|usage| &mut usage.segments, "打包段数", "旧记录打不了包"),
    ];
    for (pick, name, refused) in places {
        // 4096 的八成是 3276.8：3277 才算到。
        for (used, percent, state) in [
            (3_276, 79, "normal"),
            (3_277, 80, "degraded"),
            (4_096, 100, "degraded"),
            (6_000, 146, "degraded"),
        ] {
            let (found, detail) = read(|usage| {
                *pick(usage) = RecordUsageItem { used, limit: 4_096 };
            });
            assert_eq!(found, state, "{name} {used}");
            let said = format!("{name} {used}／4096，已到上限的 {percent}%");
            let expected = if state == "degraded" {
                format!("{said}：满了之后{refused}；已打包 0 条")
            } else {
                format!("最满的一处：{said}；已打包 0 条")
            };
            assert_eq!(detail, expected);
        }
    }
}

#[test]
fn several_full_places_lead_with_the_fullest_and_list_the_rest() {
    let (state, detail) = read(|usage| {
        usage.descriptors.used = 3_400;
        usage.replays.used = 4_000;
        usage.segments.used = 1_000;
    });
    assert_eq!(state, "degraded");
    assert_eq!(
        detail,
        "幂等重放条数 4000／4096，已到上限的 97%：满了之后备份封存、新建笔记、改名和删除会被拒；另有：正文记录条数、打包段数；已打包 0 条"
    );
}

#[test]
fn unread_and_unassembled_are_unknown_and_never_green() {
    let (state, detail) = section(RecordUsageEvidence::Unreadable);
    assert_eq!(state, "unknown");
    assert_eq!(detail, "这一次没读到写者记录的用量，状态未知");
    let (state, detail) = section(RecordUsageEvidence::Disabled);
    assert_eq!(state, "unknown");
    assert_eq!(detail, "没有装配写者，看不到写者记录的用量");
}

/// 八处全满也写得进脱敏边界的长度。
#[test]
fn every_place_full_at_once_still_fits_the_boundary() {
    let full = RecordUsageItem {
        used: u64::MAX,
        limit: u64::MAX,
    };
    let (state, detail) = section(RecordUsageEvidence::Read(RecordUsage {
        descriptors: full,
        descriptor_bytes: full,
        replays: full,
        replay_bytes: full,
        file_operations: full,
        file_operation_bytes: full,
        tombstones: full,
        segments: full,
        packs: u64::MAX,
    }));
    assert_eq!(state, "degraded");
    assert!(detail.starts_with("正文记录条数 "), "{detail}");
}
