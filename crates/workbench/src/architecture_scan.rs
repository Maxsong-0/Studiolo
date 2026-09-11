//! 严格扫描三张普通 Markdown 表；不把示例围栏或注释内容当声明。

use crate::{markdown, operations_block, table};

#[derive(Clone, Copy)]
pub(crate) enum Section {
    Nodes,
    Relations,
    Flows,
}

impl Section {
    fn headers(self) -> &'static [&'static str] {
        match self {
            Self::Nodes => &["ID", "名称", "类型", "目录", "职责"],
            Self::Relations => &["起点", "终点", "类型", "说明"],
            Self::Flows => &["流程ID", "流程名称", "步骤", "节点ID", "说明"],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Header,
    Separator,
    Rows,
    Finished,
}

struct Active {
    section: Section,
    stage: Stage,
    previous: u32,
}

impl Active {
    fn complete(&self) -> bool {
        matches!(self.stage, Stage::Rows | Stage::Finished)
    }

    fn line(
        &mut self,
        text: &str,
        number: u32,
        row: &mut impl FnMut(Section, u32, Vec<String>) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.stage == Stage::Separator && number != self.previous + 1 {
            return Err("表头与分隔行必须相邻".to_owned());
        }
        if self.stage == Stage::Rows && number != self.previous + 1 {
            self.stage = Stage::Finished;
        }
        let Some(cells) = table::literal_cells(text) else {
            if self.stage == Stage::Separator || text.contains('|') {
                return Err("表格必须使用完整的竖线边界和固定分隔行".to_owned());
            }
            if self.stage == Stage::Rows {
                self.stage = Stage::Finished;
            }
            return Ok(());
        };
        let headers = self.section.headers();
        if cells.len() != headers.len() {
            return Err("表格列数与声明契约不一致".to_owned());
        }
        match self.stage {
            Stage::Header => {
                if !cells.iter().map(String::as_str).eq(headers.iter().copied()) {
                    return Err("表头名称或列顺序与声明契约不一致".to_owned());
                }
                self.stage = Stage::Separator;
            }
            Stage::Separator => {
                if !table::is_separator(&cells) {
                    return Err("表头后缺少有效 Markdown 分隔行".to_owned());
                }
                self.stage = Stage::Rows;
            }
            Stage::Rows => row(self.section, number, cells)?,
            Stage::Finished => return Err("同一声明章节只允许一张连续表格".to_owned()),
        }
        self.previous = number;
        Ok(())
    }
}

pub(crate) fn scan(
    content: &str,
    mut row: impl FnMut(Section, u32, Vec<String>) -> Result<(), String>,
) -> Result<(), String> {
    if content.len() > 64 * 1024 {
        return Err("项目运作声明超过 64 KiB，整份无效".to_owned());
    }
    let mut fence: Option<(u8, usize)> = None;
    let mut frontmatter = false;
    let mut comment = false;
    let mut active: Option<Active> = None;
    let mut seen = [false; 3];
    for (offset, raw) in content.lines().enumerate() {
        let number = u32::try_from(offset + 1).map_err(|_| "声明行号超限".to_owned())?;
        let raw = if offset == 0 {
            raw.trim_start_matches('\u{feff}')
        } else {
            raw
        };
        if offset == 0 && raw.trim() == "---" {
            frontmatter = true;
            continue;
        }
        if frontmatter {
            if matches!(raw.trim(), "---" | "...") {
                frontmatter = false;
            }
            continue;
        }
        // 围栏优先，正文中的注释标记不能泄漏到后续真实章节。
        if let Some((marker, length)) = fence {
            if operations_block::fence_marker(raw).is_some_and(|(close, run, tail)| {
                close == marker && run >= length && tail.is_empty()
            }) {
                fence = None;
            }
            continue;
        }
        if !comment && let Some((marker, length, _)) = operations_block::fence_marker(raw) {
            fence = Some((marker, length));
            continue;
        }
        let visible = operations_block::visible_text(raw, &mut comment);
        if visible.starts_with("    ") || visible.starts_with('\t') {
            continue;
        }
        let text = visible.trim();
        let line = markdown::Line { number, text };
        if let Some((level, title)) = markdown::heading(&line) {
            if active.as_ref().is_some_and(|active| !active.complete()) {
                return Err(format!("第 {number} 行：上一声明章节缺少完整表格"));
            }
            // 本声明格式不含子章节，不能借下级标题隐藏第二张表。
            if level > 2 {
                if active.is_some() {
                    return Err(format!("第 {number} 行：声明二级章节内不允许子标题"));
                }
                continue;
            }
            active = None;
            let section = match (level, title.as_str()) {
                (2, "节点") => Some(Section::Nodes),
                (2, "关系") => Some(Section::Relations),
                (2, "任务路径") => Some(Section::Flows),
                _ => None,
            };
            if let Some(section) = section {
                let index = section as usize;
                if seen[index] {
                    return Err(format!("第 {number} 行：声明二级章节重复"));
                }
                seen[index] = true;
                active = Some(Active {
                    section,
                    stage: Stage::Header,
                    previous: 0,
                });
            }
        } else if let Some(active) = active.as_mut() {
            // 注释可屏蔽整行示例，但不能静默改写真实表格中的 ID、目录或说明。
            if text.starts_with('|') && visible != raw {
                return Err(format!("第 {number} 行：HTML 注释改变了声明表行，整份无效"));
            }
            active
                .line(text, number, &mut row)
                .map_err(|reason| format!("第 {number} 行：{reason}"))?;
        }
    }
    if frontmatter || comment || fence.is_some() {
        return Err("声明文件中的 frontmatter、注释或围栏未闭合".to_owned());
    }
    if seen.iter().any(|present| !present)
        || active.as_ref().is_some_and(|active| !active.complete())
    {
        return Err("必须包含节点、关系、任务路径三个二级章节及完整表格".to_owned());
    }
    Ok(())
}
