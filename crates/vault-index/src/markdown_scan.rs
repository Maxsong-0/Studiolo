use std::{collections::HashMap, ops::Range};

/// 标题与链接共用的块级扫描。闭合围栏必须使用同一种标记、长度不短于开栏且行尾没有非空白
/// 内容。缩进行一律保留：Obsidian 的嵌套列表项与续行默认用 tab 或四空格缩进，契约（vault-index.md
/// §3）只授权跳过围栏与行内 code；把缩进行当作缩进代码块丢弃会让嵌套列表里的 wikilink 与反链
/// 静默消失（2026-09-06 T-032 撤销该判定）。
pub(crate) fn visible_lines(body: &str, first_line: u32) -> Vec<(u32, &str)> {
    let mut visible = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    for (offset, line) in body.lines().enumerate() {
        let indent = line.bytes().take_while(|byte| *byte == b' ').count();
        let marker = if indent <= 3 && !line.starts_with('\t') {
            fence_marker(&line[indent..])
        } else {
            None
        };
        if let Some((open, length)) = fence {
            if let Some((kind, run, rest)) = marker
                && kind == open
                && run >= length
                && rest.trim().is_empty()
            {
                fence = None;
            }
            continue;
        }
        if let Some((kind, run, rest)) = marker
            && (kind != b'`' || !rest.contains('`'))
        {
            fence = Some((kind, run));
            continue;
        }
        visible.push((first_line + offset as u32, line));
    }
    visible
}

fn fence_marker(line: &str) -> Option<(u8, usize, &str)> {
    let kind = *line.as_bytes().first()?;
    if !matches!(kind, b'`' | b'~') {
        return None;
    }
    let run = line.bytes().take_while(|byte| *byte == kind).count();
    (run >= 3).then_some((kind, run, &line[run..]))
}

/// 行内 code 由等长的反引号串界定，而不是看全部反引号的奇偶；找不到等长闭合串的起始串
/// 仍是普通字面文本。返回的区间按起点升序且互不重叠，链接解析据此单调推进区间下标。
pub(crate) fn code_spans(line: &str) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = line[cursor..].find('`') {
        let start = cursor + relative;
        let run = backticks(&line[start..]);
        cursor = start + run;
        runs.push(start..cursor);
    }

    // 每个串只建立一次「右侧最近等长串」索引，未闭合开串不再反复搜索整段余文。
    // 闭串可有前导反斜杠：转义仅影响开串，保留原先的 Markdown 语义。
    let mut next_by_length = HashMap::new();
    let mut closing = vec![None; runs.len()];
    for (index, run) in runs.iter().enumerate().rev() {
        closing[index] = next_by_length.insert(run.len(), index);
    }

    let mut spans = Vec::new();
    let mut index = 0;
    while let Some(run) = runs.get(index) {
        if escaped(line, run.start) {
            index += 1;
            continue;
        }
        if let Some(end) = closing[index] {
            spans.push(run.start..runs[end].end);
            index = end + 1;
        } else {
            index += 1;
        }
    }
    spans
}

fn backticks(text: &str) -> usize {
    text.bytes().take_while(|byte| *byte == b'`').count()
}

pub(crate) fn escaped(line: &str, position: usize) -> bool {
    line.as_bytes()[..position]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

#[cfg(test)]
#[path = "markdown_scan_tests.rs"]
mod tests;
