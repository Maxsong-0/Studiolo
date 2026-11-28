//! 写入路径每次读多少、枚举多少，和散件有几条无关（T-249 G5）。开库之后第一次用到的索引可以建一次，之后不再全目录扫。
use super::*;
use crate::writer::disk::probe;

/// 散件有 `loose` 条时：新路径写一篇、改一篇已有的、改一次名，各读了几个文件、枚举了几项。
fn costs(loose: usize) -> [(u64, u64); 3] {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 3);
    renames::enable(&mut broker);
    bulk(&broker, 0..loose);
    // 先各走一遍。
    broker.mutate_recorded(note(100), metadata()).unwrap();
    flush(&mut broker);
    let warm = renames::rename(&broker, "note-0.md", "warm.md", "warm");
    renames::run(&mut broker, warm, 9000);
    probe::take();
    broker.mutate_recorded(note(101), metadata()).unwrap();
    flush(&mut broker);
    let fresh = probe::take();
    let path = logical("note-1.md");
    let expected = broker.read(&path).unwrap().revision;
    broker
        .mutate_recorded(
            Mutation::replace(path, b"changed".to_vec(), expected, "change-1"),
            metadata(),
        )
        .unwrap();
    flush(&mut broker);
    let changed = probe::take();
    let request = renames::rename(&broker, "note-2.md", "moved.md", "move");
    renames::run(&mut broker, request, 9100);
    let renamed = probe::take();
    [fresh, changed, renamed]
}

/// 原来新路径要把全部重放读一遍，每次写入都枚举请求目录，文件操作把重放读两三遍：都随散件条数涨。
#[test]
fn write_paths_read_the_same_whatever_the_number_of_loose_records() {
    let few = costs(40);
    let many = costs(4000);
    assert_eq!(few, many);
}
