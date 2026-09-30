use std::ops::Range;

use super::{backticks, code_spans, escaped};

/// 优化前的扫描作为小输入语义参照；大输入不运行这个重复后缀扫描。
fn reference_code_spans(line: &str) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = line[cursor..].find('`') {
        let start = cursor + relative;
        let run = backticks(&line[start..]);
        cursor = start + run;
        if escaped(line, start) {
            continue;
        }
        let mut end_cursor = cursor;
        while let Some(relative) = line[end_cursor..].find('`') {
            let end = end_cursor + relative;
            let closing_run = backticks(&line[end..]);
            end_cursor = end + closing_run;
            if closing_run == run {
                spans.push(start..end_cursor);
                cursor = end_cursor;
                break;
            }
        }
    }
    spans
}

#[test]
fn equal_runs_escape_parity_and_unmatched_openers_keep_their_semantics() {
    let cases = [
        ("`` a ` b `` and ` c `", vec!["`` a ` b ``", "` c `"]),
        (r"\` literal [[a]] `unclosed", vec![]),
        (r"`escaped close \` tail", vec![r"`escaped close \`"]),
        (r"\\` code `", vec!["` code `"]),
        ("` unmatched ``code``", vec!["``code``"]),
        (
            "``` unmatched ``open `short` close``",
            vec!["``open `short` close``"],
        ),
        ("中文 ``😀 ` 内文`` 后文", vec!["``😀 ` 内文``"]),
    ];
    for (line, expected) in cases {
        let spans = code_spans(line);
        assert_eq!(
            spans
                .iter()
                .map(|span| &line[span.clone()])
                .collect::<Vec<_>>(),
            expected,
            "{line:?}"
        );
        assert!(spans.windows(2).all(|pair| pair[0].end <= pair[1].start));
    }
}

#[test]
fn all_short_run_and_escape_combinations_match_the_original_scan() {
    let pieces = ["`", "``", "```", "\\", "甲", " "];
    for mut encoding in 0..pieces.len().pow(5) {
        let mut line = String::new();
        for _ in 0..5 {
            line.push_str(pieces[encoding % pieces.len()]);
            encoding /= pieces.len();
        }
        assert_eq!(code_spans(&line), reference_code_spans(&line), "{line:?}");
    }
}

#[test]
fn thousands_of_distinct_unclosed_runs_preserve_a_final_matching_span() {
    // 约 2 MiB 的确定夹具；旧实现会为 2047 个开串反复扫描后缀。
    // 新实现每字节收集一次、每串反向索引一次、正向游标只增不减，不依赖耗时阈值。
    let mut line = String::new();
    for length in 2..=2048 {
        line.extend(std::iter::repeat_n('`', length));
        line.push_str(" 文 ");
    }
    let start = line.len();
    line.push_str("`[[隐藏]]`");
    assert_eq!(code_spans(&line), vec![start..line.len()]);
}
