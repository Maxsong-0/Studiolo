use crate::{LinkResolution, OutgoingLink, WikilinkTarget, markdown_scan};

pub(crate) fn parse_outgoing(body: &str, first_line: u32) -> Vec<OutgoingLink> {
    let mut links = Vec::new();
    for (number, line) in markdown_scan::visible_lines(body, first_line) {
        let code = markdown_scan::code_spans(line);
        let mut cursor = 0;
        while let Some(relative) = line[cursor..].find("[[") {
            let brackets = cursor + relative;
            if markdown_scan::escaped(line, brackets)
                || code.iter().any(|span| span.contains(&brackets))
            {
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
