use crate::{LinkResolution, OutgoingLink, WikilinkTarget, markdown_scan};

const MAX_CONTEXT_CHARS: usize = 320;
const CONTEXT_PREFIX_CHARS: usize = 32;

/// 逐行解析可见正文里的 wikilink 与嵌入；转义的 `[[` 与落在行内 code 里的 `[[` 不算链接。
pub(crate) fn parse_outgoing(body: &str, first_line: u32) -> Vec<OutgoingLink> {
    let mut links = Vec::new();
    for (number, line) in markdown_scan::visible_lines(body, first_line) {
        let code = markdown_scan::code_spans(line);
        let short_line = line.chars().nth(MAX_CONTEXT_CHARS).is_none();
        debug_assert!(code.windows(2).all(|pair| pair[0].end <= pair[1].start));
        // 行内 code 区间按起点升序且互不重叠，`[[` 的位置又随游标严格递增：已结束的区间
        // 不可能再包含后面的 `[[`，所以下标只需单调前进，单行判定摊还 O(链接数 + 区间数)。
        let mut next_span = 0;
        let mut cursor = 0;
        let mut column_byte = 0;
        let mut column_chars = 0;
        while let Some(relative) = line[cursor..].find("[[") {
            let brackets = cursor + relative;
            while code.get(next_span).is_some_and(|span| span.end <= brackets) {
                next_span += 1;
            }
            let in_code = code
                .get(next_span)
                .is_some_and(|span| span.start <= brackets);
            if markdown_scan::escaped(line, brackets) || in_code {
                cursor = brackets + 2;
                continue;
            }
            let Some(close_relative) = line[brackets + 2..].find("]]") else {
                break;
            };
            let close = brackets + 2 + close_relative + 2;
            let token_start =
                if line[..brackets].ends_with('!') && !markdown_scan::escaped(line, brackets - 1) {
                    brackets - 1
                } else {
                    brackets
                };
            if let Ok(target) = WikilinkTarget::parse(&line[token_start..close]) {
                // 已计数的前缀不再扫描；多链接长行的列号换算总共只遍历一次字符。
                column_chars += line[column_byte..token_start].chars().count();
                column_byte = token_start;
                links.push(OutgoingLink {
                    target,
                    resolution: LinkResolution::Dangling,
                    line: number,
                    column: column_chars as u32 + 1,
                    context: link_context(line, token_start, short_line),
                });
            }
            cursor = close;
        }
    }
    links
}

/// 短行逐字保留；长行只回看最多 32 个字符，省略号也计入 320 字符预算。
fn link_context(line: &str, token_start: usize, short_line: bool) -> String {
    if short_line {
        return line.to_owned();
    }
    let start = line[..token_start]
        .char_indices()
        .rev()
        .nth(CONTEXT_PREFIX_CHARS - 1)
        .map_or(0, |(start, _)| start);
    let mut context = String::new();
    if start > 0 {
        context.push('…');
    }
    let budget = MAX_CONTEXT_CHARS - usize::from(start > 0);
    let mut characters = line[start..].chars();
    context.extend(characters.by_ref().take(budget - 1));
    if let Some(last) = characters.next() {
        context.push(if characters.next().is_some() {
            '…'
        } else {
            last
        });
    }
    context
}
