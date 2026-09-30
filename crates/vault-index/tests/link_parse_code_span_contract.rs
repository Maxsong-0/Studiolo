//! 行内 code 与 wikilink 混排的解析回归：出链的「是否落在 code 里」改为单调区间下标判定后，
//! 结果必须与改前的朴素扫描逐条一致，落在 code 区间里的 `[[` 仍一律忽略。

use std::ops::Range;

use studiolo_core::VaultPath;
use studiolo_vault_index::{InMemoryVaultIndex, MarkdownDocument, VaultIndex, WikilinkTarget};

/// 逐条对比用的出链记录；解析状态与本改动无关，不参与对比。
#[derive(Debug, PartialEq, Eq)]
struct Parsed {
    line: u32,
    column: u32,
    target: WikilinkTarget,
    context: String,
}

/// 经公开接缝重建索引，取单篇笔记的出链。
fn outgoing(body: &str) -> Vec<Parsed> {
    let path = VaultPath::parse("混排.md").expect("fixture path is valid");
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[MarkdownDocument::new(path.clone(), body)])
        .expect("index rebuild succeeds");
    index
        .document(&path)
        .expect("document query succeeds")
        .expect("document is indexed")
        .outgoing_links
        .into_iter()
        .map(|link| Parsed {
            line: link.line,
            column: link.column,
            target: link.target,
            context: link.context,
        })
        .collect()
}

/// 优化前的 `code_spans` 语义参照：等长反引号串界定行内 code。
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

fn backticks(text: &str) -> usize {
    text.bytes().take_while(|byte| *byte == b'`').count()
}

fn escaped(line: &str, position: usize) -> bool {
    line.as_bytes()[..position]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

/// 改前的朴素扫描：每个 `[[` 都线性检查整行全部 code 区间。只用于没有 frontmatter、
/// 也没有围栏的正文（这时每一行都可见，行号即下标加一）。
fn naive_outgoing(body: &str) -> Vec<Parsed> {
    let mut links = Vec::new();
    for (offset, line) in body.lines().enumerate() {
        let code = reference_code_spans(line);
        let mut cursor = 0;
        while let Some(relative) = line[cursor..].find("[[") {
            let brackets = cursor + relative;
            if escaped(line, brackets) || code.iter().any(|span| span.contains(&brackets)) {
                cursor = brackets + 2;
                continue;
            }
            let Some(close_relative) = line[brackets + 2..].find("]]") else {
                break;
            };
            let close = brackets + 2 + close_relative + 2;
            let token_start = if line[..brackets].ends_with('!') && !escaped(line, brackets - 1) {
                brackets - 1
            } else {
                brackets
            };
            if let Ok(target) = WikilinkTarget::parse(&line[token_start..close]) {
                links.push(Parsed {
                    line: offset as u32 + 1,
                    column: line[..token_start].chars().count() as u32 + 1,
                    target,
                    context: line.to_owned(),
                });
            }
            cursor = close;
        }
    }
    links
}

/// 一行放 200 个行内 code 与 200 个真链接；每个 code 区间里都藏着一个伪链接。
fn crowded_line() -> String {
    let mut line = String::from("开头");
    for index in 0..200 {
        let fence = "`".repeat(index % 3 + 1);
        // 两个以上反引号界定的 code 里放一个单反引号，考验「等长才闭合」。
        let inner = if fence.len() == 1 { "码" } else { "码 ` 码" };
        let code = if index % 5 == 0 {
            format!("{fence}{inner} [[未闭合{index}{fence}")
        } else {
            format!("{fence}{inner} [[伪{index}]]{fence}")
        };
        let segment = match index % 4 {
            // 链接紧贴 code 结尾：`[[` 恰在区间右端（开区间之外），必须算真链接。
            0 => format!(" {code}[[真{index}]]"),
            1 => format!(" {code} ![[真{index}|别名{index}]]"),
            2 => format!(" [[真{index}#小节]]{code}"),
            _ => format!(" \\[[转义{index}]] {code} 中文[[真{index}]]"),
        };
        line.push_str(&segment);
        if index == 100 {
            // 找不到等长闭合串的孤立反引号串只是字面文本，不得吞掉后面的区间。
            line.push_str(" ```````孤立");
        }
    }
    line
}

#[test]
fn two_hundred_code_spans_and_links_match_the_naive_scan() {
    let line = crowded_line();
    assert_eq!(reference_code_spans(&line).len(), 200);
    let actual = outgoing(&line);
    let expected = naive_outgoing(&line);
    assert_eq!(actual.len(), 200);
    assert_eq!(actual.len(), expected.len());
    for (index, (link, expected)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!((link.line, link.column), (expected.line, expected.column));
        assert_eq!(link.target, expected.target);
        assert!(link.context.chars().count() <= 320);
        assert!(link.context.contains(&format!("[[真{index}")));
        assert_eq!(link.target.target, format!("真{index}"));
        assert_eq!(link.target.embed, index % 4 == 1);
    }
}

#[test]
fn wikilink_brackets_inside_code_spans_stay_ignored() {
    let body = [
        "行一 `[[伪1]]` [[真1]]",
        "行二 ``含 ` 反引号 [[伪2]]`` [[真2]]``未闭合 [[真3]]",
        "行三 [[真4]]`[[伪3`]] ```[[伪4]]```[[真5]]",
        "行四 `![[伪5]]` ![[真6]]",
    ]
    .join("\n");
    let actual = outgoing(&body);
    assert_eq!(
        actual
            .iter()
            .map(|link| link.target.target.as_str())
            .collect::<Vec<_>>(),
        ["真1", "真2", "真3", "真4", "真5", "真6"]
    );
    assert!(actual[5].target.embed);
    assert_eq!(actual, naive_outgoing(&body));
}

#[test]
fn random_lines_match_the_naive_scan() {
    let alphabet = [
        '`', '`', '[', '[', ']', ']', '!', '\\', '甲', 'a', '|', '#', ' ',
    ];
    let mut state = 0x5eed_2026_0922_0126_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize
    };
    let lines = (0..400)
        .map(|_| {
            let length = next() % 80;
            // 行首固定为汉字，保证不会开出围栏或 frontmatter，与朴素扫描的前提一致。
            std::iter::once('行')
                .chain((0..length).map(|_| alphabet[next() % alphabet.len()]))
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let body = lines.join("\n");
    let ignored_in_code = lines
        .iter()
        .map(|line| {
            let code = reference_code_spans(line);
            line.match_indices("[[")
                .filter(|(position, _)| code.iter().any(|span| span.contains(position)))
                .count()
        })
        .sum::<usize>();
    let expected = naive_outgoing(&body);
    assert!(expected.len() > 50, "随机样本要有足够多的真链接");
    assert!(
        ignored_in_code > 20,
        "随机样本要有足够多落在 code 里的 `[[`"
    );
    assert_eq!(outgoing(&body), expected);
}
