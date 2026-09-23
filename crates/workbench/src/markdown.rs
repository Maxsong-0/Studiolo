use crate::SourceLocation;
pub(crate) use crate::table::tables;

#[derive(Clone, Copy)]
pub(crate) struct Line<'a> {
    pub number: u32,
    pub text: &'a str,
}

pub(crate) fn location(path: &str, line: u32) -> SourceLocation {
    SourceLocation {
        path: path.to_owned(),
        line,
    }
}

/// 保留源行号，同时排除 YAML frontmatter、HTML 注释、围栏代码与缩进代码行（四空格或 tab 开头）。
pub(crate) fn lines(content: &str) -> Vec<Line<'_>> {
    let mut result = Vec::new();
    let mut frontmatter = false;
    let mut fence: Option<(u8, usize)> = None;
    let mut comment = false;
    for (offset, raw) in content.lines().enumerate() {
        let raw = raw.trim_start_matches('\u{feff}');
        let text = raw.trim();
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
        if comment || text.starts_with("<!--") {
            comment = !text.contains("-->");
            continue;
        }
        let marker = text.as_bytes().first().copied().unwrap_or_default();
        let run = text.bytes().take_while(|byte| *byte == marker).count();
        if let Some((open, length)) = fence {
            if marker == open && run >= length && text[run..].trim().is_empty() {
                fence = None;
            }
            continue;
        }
        if matches!(marker, b'`' | b'~') && run >= 3 {
            fence = Some((marker, run));
            continue;
        }
        if raw.starts_with("    ") || raw.starts_with('\t') {
            continue;
        }
        result.push(Line {
            number: offset as u32 + 1,
            text,
        });
    }
    result
}

pub(crate) fn heading(line: &Line<'_>) -> Option<(usize, String)> {
    let level = line.text.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&level)
        || !line
            .text
            .as_bytes()
            .get(level)
            .is_some_and(u8::is_ascii_whitespace)
    {
        return None;
    }
    Some((
        level,
        clean(line.text[level..].trim().trim_end_matches('#')),
    ))
}

pub(crate) fn section<'a>(lines: &[Line<'a>], marker: char) -> Option<(u32, Vec<Line<'a>>)> {
    let (start, (level, _)) = lines.iter().enumerate().find_map(|(index, line)| {
        heading(line)
            .filter(|(_, text)| text.starts_with(marker))
            .map(|value| (index, value))
    })?;
    let end = lines[start + 1..]
        .iter()
        .position(|line| heading(line).is_some_and(|(next_level, _)| next_level <= level))
        .map_or(lines.len(), |offset| start + 1 + offset);
    Some((lines[start].number, current_lines(&lines[start + 1..end])))
}

pub(crate) fn current_lines<'a>(lines: &[Line<'a>]) -> Vec<Line<'a>> {
    let mut excluded_level = None;
    lines
        .iter()
        .copied()
        .filter(|line| {
            if let Some((level, title)) = heading(line) {
                if excluded_level.is_some_and(|excluded| level <= excluded) {
                    excluded_level = None;
                }
                if excluded_level.is_none()
                    && ["历史", "归档", "已完成", "档案沿革"]
                        .iter()
                        .any(|word| title.contains(word))
                {
                    excluded_level = Some(level);
                }
            }
            excluded_level.is_none()
        })
        .collect()
}

pub(crate) fn clean(text: &str) -> String {
    text.replace("**", "")
        .replace("__", "")
        .replace('`', "")
        .trim()
        .to_owned()
}

pub(crate) fn meaningful(text: &str) -> bool {
    !matches!(
        clean(text).to_lowercase().as_str(),
        "" | "-"
            | "—"
            | "–"
            | "无"
            | "暂无"
            | "无待办"
            | "暂无待办"
            | "待补充"
            | "待填写"
            | "todo"
            | "tbd"
            | "（空）"
            | "(empty)"
            | "..."
    )
}

pub(crate) fn status_text(text: &str) -> String {
    clean(text)
        .trim_start_matches(|character: char| {
            character.is_whitespace()
                || matches!(character,
            '\u{200d}' | '\u{20e3}' | '\u{fe0f}' | '\u{2300}'..='\u{27ff}' |
            '\u{1f000}'..='\u{1faff}')
        })
        .to_lowercase()
}

pub(crate) fn completed(text: &str) -> bool {
    let value = status_text(text);
    if value.starts_with("done-pending-restart") || value.starts_with("代码完成") {
        return false;
    }
    [
        "已完成",
        "完成",
        "全部完成",
        "done",
        "已修复",
        "已关闭",
        "已取消",
        "已废弃",
        "已被取代",
    ]
    .iter()
    .any(|prefix| value.starts_with(prefix))
        || value
            .trim_start_matches(|character: char| {
                character.is_ascii_digit()
                    || character.is_whitespace()
                    || matches!(character, '-' | '/' | ':')
            })
            .starts_with("实现与本地验收完成")
}

pub(crate) fn field(lines: &[Line<'_>], names: &[&str]) -> Option<(String, u32)> {
    for table in tables(lines) {
        for row in table.rows {
            if row.cells.len() == 2
                && names.contains(&row.cells[0].as_str())
                && meaningful(&row.cells[1])
            {
                return Some((row.cells[1].clone(), row.line));
            }
        }
    }
    for line in lines {
        let text = clean(line.text.trim_start_matches("- "));
        if let Some((key, value)) = text.split_once([':', '：'])
            && names.contains(&key.trim())
            && meaningful(value)
        {
            return Some((value.trim().to_owned(), line.number));
        }
    }
    None
}
