use crate::{LinkResolution, OutgoingLink, WikilinkTarget, markdown_scan};

/// 逐行解析可见正文里的 wikilink 与嵌入；转义的 `[[` 与落在行内 code 里的 `[[` 不算链接。
pub(crate) fn parse_outgoing(body: &str, first_line: u32) -> Vec<OutgoingLink> {
    let mut links = Vec::new();
    for (number, line) in markdown_scan::visible_lines(body, first_line) {
        let code = markdown_scan::code_spans(line);
        debug_assert!(code.windows(2).all(|pair| pair[0].end <= pair[1].start));
        // 行内 code 区间按起点升序且互不重叠，`[[` 的位置又随游标严格递增：已结束的区间
        // 不可能再包含后面的 `[[`，所以下标只需单调前进，单行判定摊还 O(链接数 + 区间数)。
        let mut next_span = 0;
        let mut cursor = 0;
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
                links.push(OutgoingLink {
                    target,
                    resolution: LinkResolution::Dangling,
                    line: number,
                    column: line[..token_start].chars().count() as u32 + 1,
                    context: line.to_owned(),
                });
            }
            cursor = close;
        }
    }
    links
}
