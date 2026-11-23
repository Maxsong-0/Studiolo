//! 名字索引的两个上限跟着同步路径预算走（BUG-047 复查）：范围并集按预算批准得了的路径集合，
//! 不能先在写者的索引这里报上限——那正是 BUG-047 的失败方式（只放宽一处，失败挪到下一处）。
//! 用例用少量长路径、深路径把索引撑满，不必真造满预算条数。
use super::*;

/// 一条 1024 字节、独占四层各 250 字节目录的路径占的索引字节：五个新前缀，折叠键与原文各记一份。
const DEEP_COST: usize = 2 * (256 + 507 + 758 + 1009 + 1024);

fn deep(i: usize) -> VaultPath {
    let [b, c, d] = ["b", "c", "d"].map(|letter| letter.repeat(250));
    let parents = format!("notes/{i:05}{}/{b}/{c}/{d}/", "a".repeat(245));
    path(&format!("{parents}{}", "x".repeat(1024 - parents.len())))
}

/// 路径下挂 `dirs` 层目录再加文件：每条带来 `dirs + 2` 个磁盘上没有的前缀。
fn nested(count: usize, dirs: usize) -> Vec<VaultPath> {
    (0..count)
        .map(|i| path(&format!("notes/{i}{}/f", "/d".repeat(dirs))))
        .collect()
}

/// 范围并集允许路径文字合计到每条预算路径 256 字节、索引字节到每条 1 KiB；
/// 写者索引原来固定 8 MiB，路径又长又深时先触顶。
#[test]
fn required_paths_within_the_union_budgets_fit_the_name_index() {
    let budget = studiolo_core::SYNC_PATH_BUDGET;
    let f = Fixture::new();
    let mut broker = f.broker();
    fs::create_dir(f.root.join("notes")).unwrap();
    let within: Vec<_> = (0..budget / 10).map(deep).collect();
    let text: usize = within.iter().map(|item| item.as_str().len()).sum();
    // 前提：文字合计、索引字节都在并集的预算以内，但索引超过原来固定的 8 MiB。
    assert!(text <= budget * 256);
    assert!(within.len() * DEEP_COST <= budget * 1024);
    assert!(within.len() * DEEP_COST > 8 * 1024 * 1024);
    let observed = broker.capture_sync_observation(&selection(), &within);
    assert!(observed.is_ok(), "必须装得进索引");
    // 索引超过名字累计上限的两倍（每条预算路径 2 KiB），照样在接触磁盘之前整体拒绝。
    let beyond: Vec<_> = (0..budget * 3 / 10).map(deep).collect();
    assert!(beyond.len() * DEEP_COST > budget * 2048);
    reset();
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &beyond),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(count(0), 0);
}

/// 必查路径里磁盘上没有的前缀也进索引。并集索引允许每条预算路径 4 个派生前缀，
/// 再加上枚举到的名字，原来固定的 66,048 项先触顶。
#[test]
fn required_prefixes_within_the_union_budget_fit_beside_enumerated_names() {
    let budget = studiolo_core::SYNC_PATH_BUDGET;
    let f = Fixture::new();
    let mut broker = f.broker();
    fs::create_dir(f.root.join("notes")).unwrap();
    let hidden = 1500;
    for i in 0..hidden {
        fs::File::create(f.root.join(format!("notes/.hidden-{i}"))).unwrap();
    }
    // 每条 33 个新前缀。前提：合计在并集的前缀预算以内，加上枚举到的隐藏名字就超过原来固定的 66,048 项。
    let within = nested((4 * budget - 600) / 33, 31);
    assert!(within.len() * 33 < 4 * budget);
    assert!(within.len() * 33 + hidden > 66_048);
    let observed = broker.capture_sync_observation(&selection(), &within);
    assert!(observed.is_ok(), "必须装得进索引");
    // 每条 63 个新前缀（64 层，单条路径的层数上限）：超过「枚举上限＋规则＋每条预算路径 4 个」，整体拒绝。
    let beyond = nested(2200, 61);
    assert!(beyond.len() * 63 > 65_536 + 512 + 4 * budget);
    reset();
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &beyond),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(count(0), 0);
}
