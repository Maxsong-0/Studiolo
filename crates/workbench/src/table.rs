use crate::markdown::{Line, clean};

pub(crate) struct Table {
    pub line: u32,
    pub headers: Vec<String>,
    pub rows: Vec<Row>,
}

pub(crate) struct Row {
    pub line: u32,
    pub cells: Vec<String>,
}

impl Table {
    pub fn column(&self, names: &[&str]) -> Option<usize> {
        self.headers.iter().position(|header| {
            names.iter().any(|name| {
                header == name
                    || header
                        .strip_prefix(name)
                        .is_some_and(|suffix| suffix.starts_with(['（', '(']))
            })
        })
    }
}

pub(crate) fn tables(lines: &[Line<'_>]) -> Vec<Table> {
    let mut result = Vec::new();
    let mut index = 0;
    while index + 1 < lines.len() {
        let headers = cells(lines[index].text);
        let separator = cells(lines[index + 1].text);
        if let (Some(headers), Some(separator)) = (headers, separator)
            && lines[index].number + 1 == lines[index + 1].number
            && headers.len() == separator.len()
            && is_separator(&separator)
        {
            let mut table = Table {
                line: lines[index].number,
                headers,
                rows: Vec::new(),
            };
            index += 2;
            while index < lines.len() {
                if lines[index].number != lines[index - 1].number + 1 {
                    break;
                }
                let Some(cells) = cells(lines[index].text) else {
                    break;
                };
                if cells != table.headers && !is_separator(&cells) {
                    table.rows.push(Row {
                        line: lines[index].number,
                        cells,
                    });
                }
                index += 1;
            }
            result.push(table);
        } else {
            index += 1;
        }
    }
    result
}

fn is_separator(cells: &[String]) -> bool {
    cells.iter().all(|cell| {
        let cell = cell.trim_matches(':');
        cell.len() >= 3 && cell.bytes().all(|byte| byte == b'-')
    })
}

fn cells(text: &str) -> Option<Vec<String>> {
    if text.len() < 2 || !text.starts_with('|') || !text.ends_with('|') {
        return None;
    }
    let text = &text[1..text.len() - 1];
    let mut result = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    let mut brackets = 0u32;
    let mut code = false;
    for character in text.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' => {
                current.push(character);
                escaped = true;
            }
            '[' if !code => {
                brackets += 1;
                current.push(character);
            }
            ']' if !code => {
                brackets = brackets.saturating_sub(1);
                current.push(character);
            }
            '`' => {
                code = !code;
                current.push(character);
            }
            '|' if brackets == 0 && !code => {
                result.push(clean(&current));
                current.clear();
            }
            _ => current.push(character),
        }
    }
    result.push(clean(&current));
    Some(result)
}
