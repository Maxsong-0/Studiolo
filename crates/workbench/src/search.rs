//! 在现有的内存 Markdown 代次上做 ripgrep 字面匹配。

use std::{collections::BTreeMap, ops::Bound};

use grep_matcher::Matcher as _;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, SearcherBuilder, sinks::UTF8};
use studiolo_core::VaultPath;

use crate::search_dto::{
    SearchError, SearchHit, SearchHitKind, SearchMode, SearchQuery, SearchResults,
};
use crate::{DocumentView, ProjectSummary};

const MAX_HITS: usize = 100;
const MAX_SNIPPET_CHARS: usize = 320;

pub(crate) fn search(
    documents: &BTreeMap<VaultPath, DocumentView>,
    projects: &[ProjectSummary],
    query: &SearchQuery,
) -> Result<SearchResults, SearchError> {
    let project_prefix = validate(projects, query)?;
    let text = query.text.trim();
    if query.mode == SearchMode::FullText && text.is_empty() {
        return Ok(SearchResults::default());
    }
    let fragments = match query.mode {
        SearchMode::FullText => vec![text],
        // 中文输入法在中文标点模式下敲出的是全角空格 U+3000；按任意 Unicode 空白分片。
        SearchMode::Path => text
            .split(char::is_whitespace)
            .filter(|part| !part.is_empty())
            .collect(),
    };
    let matchers = fragments
        .into_iter()
        .map(literal)
        .collect::<Result<Vec<_>, _>>()?;
    let mut hits = Vec::new();
    for (path, document) in scoped_documents(documents, project_prefix.as_deref()) {
        if let Some(position) = path_match(&matchers, path.as_str())? {
            hits.push(hit(
                document,
                1,
                SearchHitKind::Path,
                excerpt(path.as_str(), position),
            ));
        }
        if hits.len() > MAX_HITS {
            break;
        }
        if query.mode == SearchMode::FullText {
            content_hits(&matchers[0], document, &mut hits)?;
        }
        if hits.len() > MAX_HITS {
            break;
        }
    }
    let truncated = hits.len() > MAX_HITS;
    hits.truncate(MAX_HITS);
    Ok(SearchResults { hits, truncated })
}

/// 有序映射直接定位项目的起点；扫到相邻项目即停止，不遍历其它项目的文档。
fn scoped_documents<'a>(
    documents: &'a BTreeMap<VaultPath, DocumentView>,
    prefix: Option<&'a str>,
) -> impl Iterator<Item = (&'a VaultPath, &'a DocumentView)> {
    let start = prefix.map_or(Bound::Unbounded, Bound::Included);
    documents
        .range::<str, _>((start, Bound::Unbounded))
        .take_while(move |(path, _)| prefix.is_none_or(|prefix| path.as_str().starts_with(prefix)))
}

fn validate(
    projects: &[ProjectSummary],
    query: &SearchQuery,
) -> Result<Option<String>, SearchError> {
    if query.text.len() > 1024
        || query.text.chars().count() > 256
        || query.text.chars().any(char::is_control)
    {
        return Err(SearchError::InvalidQuery);
    }
    match &query.project {
        Some(project) if projects.iter().any(|known| known.id == *project) => {
            Ok(Some(format!("{project}/")))
        }
        Some(_) => Err(SearchError::InvalidProject),
        None => Ok(None),
    }
}

fn literal(text: &str) -> Result<RegexMatcher, SearchError> {
    RegexMatcherBuilder::new()
        .fixed_strings(true)
        .case_insensitive(true)
        .unicode(true)
        .line_terminator(Some(b'\n'))
        .build(text)
        .map_err(|error| SearchError::Engine(error.to_string()))
}

fn path_match(matchers: &[RegexMatcher], path: &str) -> Result<Option<usize>, SearchError> {
    let mut first = None;
    for matcher in matchers {
        let Some(found) = matcher
            .find(path.as_bytes())
            .map_err(|error| SearchError::Engine(error.to_string()))?
        else {
            return Ok(None);
        };
        first = Some(first.map_or(found.start(), |position: usize| position.min(found.start())));
    }
    // 空的路径查询直接列举这同一个文档集合，不另建第二份索引。
    Ok(Some(first.unwrap_or(0)))
}

fn content_hits(
    matcher: &RegexMatcher,
    document: &DocumentView,
    hits: &mut Vec<SearchHit>,
) -> Result<(), SearchError> {
    SearcherBuilder::new()
        .line_number(true)
        .multi_line(false)
        .bom_sniffing(false)
        .binary_detection(BinaryDetection::none())
        .build()
        .search_slice(
            matcher,
            document.content.as_bytes(),
            UTF8(|number, line| {
                let number = u32::try_from(number).map_err(|_| {
                    std::io::Error::other("source line number exceeds the Interface limit")
                })?;
                let line = line.trim_end_matches(['\r', '\n']);
                if let Some(found) = matcher
                    .find(line.as_bytes())
                    .map_err(std::io::Error::other)?
                {
                    hits.push(hit(
                        document,
                        number,
                        SearchHitKind::Content,
                        excerpt(line, found.start()),
                    ));
                }
                // 先观察到第 101 条结果再停；恰好 100 条不算截断。
                Ok(hits.len() <= MAX_HITS)
            }),
        )
        .map_err(|error| SearchError::Engine(error.to_string()))
}

fn hit(document: &DocumentView, line: u32, kind: SearchHitKind, snippet: String) -> SearchHit {
    SearchHit {
        path: document.path.clone(),
        title: bounded_title(&document.title),
        line,
        snippet,
        kind,
    }
}

fn bounded_title(title: &str) -> String {
    let mut characters = title.char_indices();
    if let Some((end, _)) = characters.nth(319)
        && characters.next().is_some()
    {
        return format!("{}…", &title[..end]);
    }
    title.to_owned()
}

fn excerpt(text: &str, first_match: usize) -> String {
    // 保留首个匹配之前至多 32 个字符。最长 256 字符的字面量连同两端省略号，仍放得进
    // 320 字符的预算。只收集这个小窗口，绝不收集整条长行。
    let start = text[..first_match]
        .char_indices()
        .rev()
        .nth(31)
        .map_or(0, |(offset, _)| offset);
    let leading = usize::from(start > 0);
    let budget = MAX_SNIPPET_CHARS - leading - 1;
    let end = text[start..]
        .char_indices()
        .nth(budget)
        .map_or(text.len(), |(offset, _)| start + offset);
    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    snippet.push_str(&text[start..end]);
    if end < text.len() {
        snippet.push('…');
    }
    snippet
}

#[cfg(test)]
#[path = "search_scope_tests.rs"]
mod tests;
