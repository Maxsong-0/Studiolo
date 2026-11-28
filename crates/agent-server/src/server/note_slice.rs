//! 一篇笔记里要哪一段：整篇、行段、某个标题的一节，或只要大纲。只做纯计算，不读文件、不管权限。
//!
//! 行的数法和 search 报的行号一致：按 `\n` 切，原来的换行一个字节不动。
//! 哪些行算标题，和读模型的大纲（vault-index 的 `markdown`、`markdown_scan`）是同一套规则；
//! 那边没有「不限条数、带节末行」的接口，这里自己扫一遍，两套不许走岔由契约用例逐条对照钉住。

/// 标题级别最多到 6。
const MAX_LEVEL: usize = 6;

/// 要哪一段；构造出来的都是写法合格的。
#[derive(Debug)]
pub(super) enum NotePart {
    /// 整篇，一个字节不加。
    Whole,
    /// 从第 `first` 行到第 `last` 行，两端都含，从 1 起。
    Lines { first: usize, last: usize },
    /// 一个标题到下一个同级或更高级标题之前。
    Heading(HeadingQuery),
    /// 全部标题和各节的起止行号。
    Outline,
}

/// 为什么给不出这一段；措辞固定，不带笔记内容，也不回显参数。
#[derive(Debug)]
pub(super) enum PartError {
    /// 同时要了两种以上。
    MoreThanOne,
    /// 行段不是 `a-b`。
    MalformedLines,
    /// 标题是空的。
    EmptyHeading,
    /// 行段超出这一篇。
    LinesOutOfRange {
        /// 这一篇共几行。
        total: usize,
    },
    /// 没有这个标题。
    HeadingNotFound,
}

impl std::fmt::Display for PartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MoreThanOne => formatter.write_str("lines、heading、outline 一次只能给一个"),
            Self::MalformedLines => {
                formatter.write_str("lines 只能写成 a-b（从 1 起，两端都含，a 不大于 b）")
            }
            Self::EmptyHeading => formatter.write_str("heading 不能为空"),
            Self::LinesOutOfRange { total } => write!(formatter, "行段越界：这篇共 {total} 行"),
            Self::HeadingNotFound => formatter.write_str("没有这个标题"),
        }
    }
}

/// 切好的一段：先一行说明（整篇没有），再原文或大纲。
#[derive(Debug)]
pub(super) struct Slice<'a> {
    /// 说明和大纲，自带结尾换行；整篇时是空的。
    pub(super) head: String,
    /// 原文里的那一段。
    pub(super) body: &'a str,
}

impl NotePart {
    /// 从工具参数解析；三样一次只能给一个。
    pub(super) fn parse(
        lines: Option<&str>,
        heading: Option<&str>,
        outline: bool,
    ) -> Result<Self, PartError> {
        match (lines, heading, outline) {
            (None, None, false) => Ok(Self::Whole),
            (Some(range), None, false) => parse_range(range),
            (None, Some(query), false) => HeadingQuery::parse(query).map(Self::Heading),
            (None, None, true) => Ok(Self::Outline),
            _ => Err(PartError::MoreThanOne),
        }
    }

    /// 从正文里切出这一段。
    pub(super) fn cut<'a>(&self, content: &'a str) -> Result<Slice<'a>, PartError> {
        match self {
            Self::Whole => Ok(Slice {
                head: String::new(),
                body: content,
            }),
            Self::Lines { first, last } => {
                let (body, total) = lines_between(content, *first, *last);
                let body = body.ok_or(PartError::LinesOutOfRange { total })?;
                Ok(Slice {
                    head: format!("# 行 {first}-{last} · 共 {total} 行\n"),
                    body,
                })
            }
            Self::Heading(query) => {
                let outline = Outline::of(content);
                let (section, same_name) = query.find(&outline.sections)?;
                let (first, last, total) = (section.first, section.last, outline.total);
                let more = if same_name > 1 {
                    format!(" · 同名标题 {same_name} 处，这是第 1 处，其余用 outline 查行号")
                } else {
                    String::new()
                };
                Ok(Slice {
                    head: format!("# 行 {first}-{last} · 共 {total} 行{more}\n"),
                    body: lines_between(content, first, last).0.unwrap_or_default(),
                })
            }
            Self::Outline => {
                let outline = Outline::of(content);
                let mut head = format!(
                    "# 大纲 · {} 节 · 共 {} 行\n",
                    outline.sections.len(),
                    outline.total
                );
                for section in &outline.sections {
                    head.push_str(&format!(
                        "{}-{}\t{} {}\n",
                        section.first,
                        section.last,
                        "#".repeat(section.level),
                        section.text
                    ));
                }
                Ok(Slice { head, body: "" })
            }
        }
    }
}

/// 只认两个十进制数中间一个减号：不许空格、正负号、省略一头。
fn parse_range(range: &str) -> Result<NotePart, PartError> {
    let number = |digits: &str| {
        (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| digits.parse::<usize>().ok())
            .flatten()
    };
    range
        .split_once('-')
        .and_then(|(first, last)| Some((number(first)?, number(last)?)))
        .filter(|(first, last)| *first >= 1 && first <= last)
        .map(|(first, last)| NotePart::Lines { first, last })
        .ok_or(PartError::MalformedLines)
}

/// 第 `first` 到 `last` 行的原文（越界是 `None`），和总行数。
fn lines_between(content: &str, first: usize, last: usize) -> (Option<&str>, usize) {
    let (mut start, mut end, mut offset, mut total) = (None, None, 0, 0);
    for line in content.split_inclusive('\n') {
        total += 1;
        if total == first {
            start = Some(offset);
        }
        offset += line.len();
        if total == last {
            end = Some(offset);
        }
    }
    let body = start
        .zip(end)
        .and_then(|(start, end)| content.get(start..end));
    (body, total)
}

/// 一个标题和它那一节的起止行。
#[derive(Debug)]
struct Section<'a> {
    level: usize,
    text: &'a str,
    first: usize,
    last: usize,
}

#[derive(Debug)]
struct Outline<'a> {
    sections: Vec<Section<'a>>,
    total: usize,
}

impl<'a> Outline<'a> {
    fn of(content: &'a str) -> Self {
        let mut sections: Vec<Section<'a>> = Vec::new();
        // 还没遇到下一个同级或更高级标题的节；级别从外到里递增。
        let mut open: Vec<usize> = Vec::new();
        let mut scan = Scan::default();
        let mut total = 0;
        for line in content.split_inclusive('\n') {
            total += 1;
            let Some((level, text)) = scan.visible(total, line).and_then(heading) else {
                continue;
            };
            while let Some(index) = open.pop() {
                if sections[index].level < level {
                    open.push(index);
                    break;
                }
                sections[index].last = total - 1;
            }
            open.push(sections.len());
            sections.push(Section {
                level,
                text,
                first: total,
                last: total,
            });
        }
        for index in open {
            sections[index].last = total;
        }
        Self { sections, total }
    }
}

/// 要找的标题；带了 `#` 就连级别一起核。
#[derive(Debug)]
pub(super) struct HeadingQuery {
    level: Option<usize>,
    text: String,
}

impl HeadingQuery {
    fn parse(query: &str) -> Result<Self, PartError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(PartError::EmptyHeading);
        }
        let marks = query.bytes().take_while(|byte| *byte == b'#').count();
        let marked = (1..=MAX_LEVEL).contains(&marks)
            && query
                .as_bytes()
                .get(marks)
                .is_some_and(u8::is_ascii_whitespace);
        Ok(Self {
            level: marked.then_some(marks),
            text: query[if marked { marks } else { 0 }..].trim().to_owned(),
        })
    }

    /// 第一处，和一共几处。先按原样找；没有再去掉结尾的井号找（`## C#` 的标题文字是 `C`）。
    fn find<'s, 'a>(
        &self,
        sections: &'s [Section<'a>],
    ) -> Result<(&'s Section<'a>, usize), PartError> {
        [self.text.as_str(), closed(&self.text)]
            .into_iter()
            .find_map(|text| {
                let mut found = sections.iter().filter(|section| {
                    section.text == text && self.level.is_none_or(|level| level == section.level)
                });
                found.next().map(|first| (first, found.count() + 1))
            })
            .ok_or(PartError::HeadingNotFound)
    }
}

/// 去掉标题结尾可有可无的井号。
fn closed(text: &str) -> &str {
    text.trim().trim_end_matches('#').trim()
}

/// 这一行是不是标题：级别和文字。
fn heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=MAX_LEVEL).contains(&level)
        || !trimmed
            .as_bytes()
            .get(level)
            .is_some_and(u8::is_ascii_whitespace)
    {
        return None;
    }
    let text = closed(&trimmed[level..]);
    (!text.is_empty()).then_some((level, text))
}

/// 逐行扫描的状态：frontmatter 和代码围栏里的行不看。
#[derive(Debug, Default)]
struct Scan {
    frontmatter: bool,
    /// 开着的围栏：用的字符和长度。
    fence: Option<(u8, usize)>,
}

impl Scan {
    /// 这一行若是正文就给出去掉换行的它；frontmatter、围栏和围栏里的行不给。
    /// 没闭合的 frontmatter 一直开到文末：读模型里这样的笔记也没有标题。
    fn visible<'a>(&mut self, number: usize, line: &'a str) -> Option<&'a str> {
        // 行尾的回车不用管：下面每一处比较都先去掉行尾空白。
        let line = line.strip_suffix('\n').unwrap_or(line);
        if number == 1 {
            let line = line.strip_prefix('\u{feff}').unwrap_or(line);
            if line.trim_end_matches('\r') == "---" {
                self.frontmatter = true;
                return None;
            }
            return self.outside_fence(line);
        }
        if self.frontmatter {
            self.frontmatter = !matches!(line.trim_end_matches('\r'), "---" | "...");
            return None;
        }
        self.outside_fence(line)
    }

    fn outside_fence<'a>(&mut self, line: &'a str) -> Option<&'a str> {
        let marker = fence_marker(line);
        if let Some((kind, length)) = self.fence {
            if let Some((other, run, rest)) = marker
                && other == kind
                && run >= length
                && rest.trim().is_empty()
            {
                self.fence = None;
            }
            return None;
        }
        // 反引号围栏的说明里不能再有反引号，否则它只是一段行内代码。
        if let Some((kind, run, rest)) = marker
            && (kind != b'`' || !rest.contains('`'))
        {
            self.fence = Some((kind, run));
            return None;
        }
        Some(line)
    }
}

/// 围栏行：字符、长度、后面的字。缩进四格以上或制表符开头的不算。
fn fence_marker(line: &str) -> Option<(u8, usize, &str)> {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let line = &line[indent..];
    let kind = *line.as_bytes().first()?;
    if !matches!(kind, b'`' | b'~') {
        return None;
    }
    let run = line.bytes().take_while(|byte| *byte == kind).count();
    (run >= 3).then(|| (kind, run, &line[run..]))
}
