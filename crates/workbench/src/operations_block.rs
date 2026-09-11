//! 局部扫描章节、围栏与注释；复用标题语法，不改变通用 Markdown 解析。

use crate::markdown;

pub(crate) struct DeclarationBlock<'a> {
    pub(crate) line: u32,
    pub(crate) lines: Vec<(u32, &'a str)>,
}

pub(crate) fn extract(content: &str) -> Result<Option<DeclarationBlock<'_>>, String> {
    let mut fence: Option<(u8, usize, bool, u32)> = None;
    let mut frontmatter = false;
    let mut comment = false;
    let mut selected_section = false;
    let mut block = Vec::new();
    let mut bytes = 0_usize;
    for (offset, raw) in content.lines().enumerate() {
        let number = u32::try_from(offset + 1).map_err(|_| "声明行号超限".to_owned())?;
        let text = raw.trim_start_matches('\u{feff}').trim();
        if offset == 0 && text == "---" {
            frontmatter = true;
            continue;
        }
        if frontmatter {
            if matches!(text, "---" | "...") {
                frontmatter = false;
            }
            continue;
        }
        // 围栏正文中的 HTML 注释只是字面文本，绝不能污染后续章节状态。
        if let Some((marker, length, selected, line)) = fence {
            if fence_marker(raw).is_some_and(|(close, run, tail)| {
                close == marker && run >= length && tail.is_empty()
            }) {
                if selected {
                    return Ok(Some(DeclarationBlock { line, lines: block }));
                }
                fence = None;
            } else if selected {
                bytes = bytes.saturating_add(raw.len()).saturating_add(1);
                if bytes > 64 * 1024 {
                    return Err("目录声明块超过 64 KiB，整块不可用".to_owned());
                }
                block.push((number, raw));
            }
            continue;
        }
        // 只有原行的真实围栏能开启代码块；不能清掉行前注释后造出一个围栏。
        if !comment && let Some((marker, length, tail)) = fence_marker(raw) {
            fence = Some((marker, length, selected_section && tail == "text", number));
            continue;
        }
        let visible = visible_text(raw, &mut comment);
        if visible.starts_with("    ") || visible.starts_with('\t') {
            continue;
        }
        let line = markdown::Line {
            number,
            text: visible.trim(),
        };
        if let Some((level, title)) = markdown::heading(&line) {
            if selected_section && level <= 2 {
                return Ok(None);
            }
            selected_section |= level == 2
                && title
                    .strip_prefix("代码仓库")
                    .is_some_and(|tail| tail.is_empty() || tail.starts_with([' ', '（', '(']));
        }
    }
    if fence.is_some_and(|(_, _, selected, _)| selected) {
        return Err("目录声明的 text 围栏未闭合，整块不可用".to_owned());
    }
    Ok(None)
}

/// 注释替换为空格以保留缩进；行内代码中的注释标记同样只是文本。
fn visible_text(mut raw: &str, comment: &mut bool) -> String {
    let mut result = String::new();
    while !raw.is_empty() {
        if *comment {
            let length = raw.find("-->").map_or(raw.len(), |offset| offset + 3);
            *comment = !raw[..length].ends_with("-->");
            result.extend(std::iter::repeat_n(' ', length));
            raw = &raw[length..];
        } else if raw.starts_with("<!--") {
            *comment = true;
        } else if raw.starts_with('`') {
            let run = raw.bytes().take_while(|byte| *byte == b'`').count();
            let length = inline_code_end(&raw[run..], run).map_or(run, |end| run + end);
            result.push_str(&raw[..length]);
            raw = &raw[length..];
        } else {
            let length = match raw.find(['<', '`']) {
                Some(0) => raw.chars().next().map_or(0, char::len_utf8),
                Some(offset) => offset,
                None => raw.len(),
            };
            result.push_str(&raw[..length]);
            raw = &raw[length..];
        }
    }
    result
}

fn inline_code_end(mut raw: &str, length: usize) -> Option<usize> {
    let mut consumed = 0;
    while let Some(offset) = raw.find('`') {
        let run = raw[offset..]
            .bytes()
            .take_while(|byte| *byte == b'`')
            .count();
        consumed += offset + run;
        if run == length {
            return Some(consumed);
        }
        raw = &raw[offset + run..];
    }
    None
}

fn fence_marker(raw: &str) -> Option<(u8, usize, &str)> {
    let text = raw.trim_start_matches(' ');
    if raw.len() - text.len() > 3 {
        return None;
    }
    let marker = *text.as_bytes().first()?;
    let length = text.bytes().take_while(|byte| *byte == marker).count();
    (matches!(marker, b'`' | b'~') && length >= 3).then(|| (marker, length, text[length..].trim()))
}
