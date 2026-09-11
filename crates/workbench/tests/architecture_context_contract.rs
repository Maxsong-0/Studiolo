//! Markdown 上下文必须完整，伪声明与额外表格不得产生看似有效的部分图。

#[path = "support/architecture.rs"]
mod support;

use support::*;

#[test]
fn fenced_commented_frontmatter_and_indented_examples_cannot_supply_required_tables() {
    let content = declaration(NODE, "", "");
    for fake in [
        format!("```markdown\n{content}```\n"),
        format!("~~~~markdown\n{content}~~~~\n"),
        format!("<!--\n{content}-->\n"),
        format!(
            "---\nexample: |\n{}\n---\n",
            content
                .lines()
                .map(|line| format!("  {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        content
            .lines()
            .map(|line| format!("    {line}\n"))
            .collect(),
    ] {
        invalid(&fake);
    }
}

#[test]
fn excluded_examples_do_not_corrupt_real_tables_and_crlf_source_lines() {
    let prefix = "\u{feff}---\nlabel: 示例\n---\n```markdown\n<!--\n## 节点\n```\n说明 `<!--` 是字面量。\n说明 <!--\n## 节点\n-->\n";
    let content = format!("{prefix}{}", declaration(NODE, "", "")).replace('\n', "\r\n");
    let value = architecture(&content);
    assert_eq!(value.status, "ready");
    assert_eq!(value.nodes[0].source.path, PATH);
    assert_eq!(value.nodes[0].source.line, 16);
}

#[test]
fn duplicate_sections_extra_tables_and_repeated_header_or_separator_reject_all_rows() {
    let content = declaration(NODE, "", "");
    for bad in [
        format!("{content}\n## 节点\n{NODE_HEADER}{NODE}"),
        content.replace(NODE, &format!("{NODE}\n{NODE_HEADER}{NODE}")),
        content.replace(NODE, &format!("{NODE}{NODE_HEADER}")),
        content.replace(NODE, &format!("{NODE}|---|---|---|---|---|\n")),
        content.replace(NODE, &format!("{NODE}\n### 补充\n{NODE_HEADER}{NODE}")),
        content.replace("## 节点\n", "## 节点\n### 说明\n"),
    ] {
        invalid(&bad);
    }
}

#[test]
fn exact_headings_headers_and_column_counts_are_required() {
    let content = declaration(NODE, "", "");
    for bad in [
        content.replace("## 节点", "### 节点"),
        content.replace("## 节点", "## 节点补充"),
        content.replace("| ID | 名称 |", "| 名称 | ID |"),
        content.replace("| 接收查询 |", "| 接收查询 | 额外列 |"),
        content.replace("| 接口 | src/search", "| src/search"),
        content.replace(NODE, "| query | 检索接口 | 接口 | - | 缺尾线\n"),
        content.replace(
            NODE,
            &format!("{NODE}catalog | 索引 | 模块 | - | 缺首线 |\n"),
        ),
    ] {
        invalid(&bad);
    }
}

#[test]
fn table_segments_cannot_be_joined_across_comments_fences_or_blank_lines() {
    let content = declaration(NODE, "", "");
    for gap in [
        "\n",
        "<!-- 隔断 -->\n",
        "```text\n忽略\n```\n",
        "    缩进代码\n",
    ] {
        let split_header = content.replacen(
            "|---|---|---|---|---|",
            &format!("{gap}|---|---|---|---|---|"),
            1,
        );
        invalid(&split_header);
        let split_row = content.replace(
            NODE,
            &format!("{NODE}{gap}| catalog | 索引 | 模块 | - | 存储 |\n"),
        );
        invalid(&split_row);
    }
}

#[test]
fn unclosed_constructs_and_missing_tables_cannot_publish_partial_graphs() {
    let content = declaration(NODE, "", "");
    for suffix in ["<!--", "```markdown", "~~~~"] {
        invalid(&format!("{content}\n{suffix}\n"));
    }
    invalid(&content.replace(FLOW_HEADER, ""));
    invalid(&content.replace(RELATION_HEADER, "```md\n| 示例 |\n```\n"));
    invalid(&content.replace("## 关系", "<!-- ## 关系 -->"));
}

#[test]
fn html_comments_cannot_rewrite_actual_ids_directories_or_descriptions() {
    for node in [
        NODE.replace("query", "que<!-- 改写 -->ry"),
        NODE.replace("src/search", "src/search<!-- 改写 -->"),
        NODE.replace("接收查询", "接收<!-- 改写 -->查询"),
        NODE.replace("接收查询", "<!-- 被删除 -->接收查询"),
    ] {
        invalid(&declaration(&node, "", ""));
    }
    let literal = NODE.replace("接收查询", "`<!-- 字面量 -->` 与 <b>普通文本</b>");
    let value = architecture(&declaration(&literal, "", ""));
    assert_eq!(value.status, "ready");
    assert_eq!(
        value.nodes[0].description,
        "`<!-- 字面量 -->` 与 <b>普通文本</b>"
    );
}

#[test]
fn comment_removal_cannot_create_a_heading_or_a_table_from_inline_html() {
    let content = declaration(NODE, "", "");
    invalid(&content.replace("## 节点", "<!-- 注释 --> ## 节点"));
    invalid(&content.replace(NODE_HEADER, &format!("<!-- 注释 --> {NODE_HEADER}")));
    let prefix = "说明 <!--\n## 节点\n| ID | 名称 | 类型 | 目录 | 职责 |\n|---|---|---|---|---|\n| fake | 伪 | 模块 | - | 伪 |\n-->\n";
    let value = architecture(&format!("{prefix}{content}"));
    assert_eq!(value.status, "ready");
    assert_eq!(value.nodes.len(), 1);
    assert_eq!(value.nodes[0].id, "query");
}
