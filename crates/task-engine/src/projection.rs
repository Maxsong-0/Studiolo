//! 任务页投影只替换四个顶层标量，其他字节归作者所有。

pub(crate) mod key;

use std::collections::BTreeSet;

use serde_json::{Value, json};
use studiolo_core::TaskId;

use crate::TaskSpec;

const FIELDS: [&str; 4] = ["status", "active_attempt", "last_attempt", "input_snapshot"];

pub(crate) fn validate_task_page(content: &[u8], task_id: TaskId) -> Result<(), String> {
    let text = std::str::from_utf8(content).map_err(|_| "任务页不是 UTF-8")?;
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return Err("任务页缺少 frontmatter".to_owned());
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !closed {
        return Err("任务页 frontmatter 未闭合".to_owned());
    }
    let parsed: Value = serde_saphyr::from_str(&yaml).map_err(|_| "任务页 frontmatter 无效")?;
    if parsed.get("type").and_then(Value::as_str) != Some("task")
        || parsed.get("id").and_then(Value::as_str) != Some(task_id.to_string().as_str())
    {
        return Err("任务页身份不匹配".to_owned());
    }
    Ok(())
}

pub(crate) fn render_task_page(
    task_id: TaskId,
    spec: &TaskSpec,
    created: &str,
) -> Result<Vec<u8>, String> {
    let mut page = String::from("---\n");
    let fields = [
        ("type", json!("task")),
        ("id", json!(task_id.to_string())),
        ("created", json!(created)),
        ("status", json!("draft")),
        ("requested_action", Value::Null),
        ("active_attempt", Value::Null),
        ("last_attempt", Value::Null),
        ("input_snapshot", Value::Null),
    ];
    for (key, value) in fields {
        page.push_str(&format!("{key}: {value}\n"));
    }
    let definition = serde_json::to_value(spec).map_err(|_| "任务定义无法序列化")?;
    for (key, value) in definition.as_object().ok_or("任务定义不是对象")? {
        let key = if key == "timeout" {
            "attempt_timeout_seconds"
        } else {
            key
        };
        // JSON 的字符串、序列和对象都是合法 YAML；不经 YAML 重排既有文件。
        page.push_str(&format!("{key}: {value}\n"));
    }
    page.push_str("---\n\n");
    if let Some(title) = &spec.title {
        page.push_str(&format!("# {title}\n\n"));
    }
    if let Some(instruction) = &spec.instruction {
        page.push_str(instruction);
        page.push('\n');
    }
    Ok(page.into_bytes())
}

pub(crate) fn apply_projection(content: &[u8], projection: &Value) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(content).map_err(|_| "任务页不是 UTF-8")?;
    let lines: Vec<_> = text.split_inclusive('\n').collect();
    if lines
        .first()
        .map(|line| line.trim_end_matches(['\r', '\n']))
        != Some("---")
    {
        return Err("任务页缺少 frontmatter".to_owned());
    }
    let close = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| line.trim_end_matches(['\r', '\n']) == "---")
        .map(|(index, _)| index)
        .ok_or("任务页 frontmatter 未闭合")?;
    let yaml = lines[1..close].concat();
    let parsed: Value = serde_saphyr::from_str(&yaml).map_err(|_| "任务页 frontmatter 无效")?;
    let mapping = parsed.as_object().ok_or("任务页 frontmatter 不是映射")?;
    for key in FIELDS {
        let value = projection.get(key).ok_or("投影字段不完整")?;
        if !value.is_string() && !value.is_null() {
            return Err("投影值必须是字符串或 null".to_owned());
        }
    }

    let newline = if lines[0].ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut seen = BTreeSet::new();
    let mut output = lines[0].to_owned();
    for line in &lines[1..close] {
        let body = line.trim_end_matches(['\r', '\n']);
        let ending = &line[body.len()..];
        let Some((raw_key, colon)) = top_level_key(body) else {
            output.push_str(line);
            continue;
        };
        let canonical = if raw_key == "状态" {
            "status"
        } else {
            &raw_key
        };
        if !FIELDS.contains(&canonical) {
            output.push_str(line);
            continue;
        }
        if !seen.insert(canonical.to_owned()) {
            return Err("任务页投影键重复或存在别名冲突".to_owned());
        }
        let after_colon = &body[colon + 1..];
        let spacing = after_colon.len() - after_colon.trim_start_matches([' ', '\t']).len();
        let value_part = &after_colon[spacing..];
        let comment = comment_offset(value_part).unwrap_or(value_part.len());
        let raw_value = value_part[..comment].trim_end();
        if raw_value.starts_with(['&', '*', '!']) {
            return Err("投影字段不能含 YAML anchor、alias 或 tag".to_owned());
        }
        // 多行标量或复杂 YAML 无法安全做单行补丁时，保留 outbox 等待人工修正。
        let scalar: Value = if raw_value.is_empty() {
            Value::Null
        } else {
            serde_saphyr::from_str(raw_value).map_err(|_| "投影字段不是单行标量")?
        };
        if (!scalar.is_string() && !scalar.is_null()) || mapping.get(&raw_key) != Some(&scalar) {
            return Err("投影字段不是独立的顶层单行标量".to_owned());
        }
        output.push_str(&body[..colon + 1]);
        if spacing == 0 {
            output.push(' ');
        } else {
            output.push_str(&after_colon[..spacing]);
        }
        output.push_str(&projection[canonical].to_string());
        let suffix = &value_part[raw_value.len()..];
        if suffix.starts_with('#') {
            output.push(' ');
        }
        output.push_str(suffix);
        output.push_str(ending);
    }
    for key in FIELDS {
        // flow mapping、跨行键等未被行扫描器识别时拒绝，不能补出第二个同名键。
        if !seen.contains(key) {
            if mapping.contains_key(key) || (key == "status" && mapping.contains_key("状态")) {
                return Err("任务页包含无法安全补丁的投影键".to_owned());
            }
            output.push_str(&format!("{key}: {}{newline}", projection[key]));
        }
    }
    output.push_str(&lines[close..].concat());
    let patched_yaml = output
        .lines()
        .skip(1)
        .take_while(|line| *line != "---")
        .collect::<Vec<_>>()
        .join("\n");
    serde_saphyr::from_str::<Value>(&patched_yaml)
        .map_err(|_| "投影后 frontmatter 无效，已拒绝写入")?;
    Ok(output.into_bytes())
}

fn top_level_key(line: &str) -> Option<(String, usize)> {
    if line.starts_with(char::is_whitespace) || line.starts_with('#') {
        return None;
    }
    let colon = outside_quotes(line, ':')?;
    let raw = line[..colon].trim();
    let key: String = serde_saphyr::from_str(raw).ok()?;
    Some((key, colon))
}

fn comment_offset(value: &str) -> Option<usize> {
    outside_quotes(value, '#')
}

fn outside_quotes(value: &str, target: char) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut chars = value.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (Some('"'), '\\') => escaped = true,
            (Some('\''), '\'') if chars.peek().is_some_and(|(_, next)| *next == '\'') => {
                chars.next();
            }
            (Some(active), current) if active == current => quote = None,
            (None, '\'' | '"') if index == 0 => quote = Some(ch),
            (None, current)
                if current == target
                    && (target != '#'
                        || index == 0
                        || value[..index].ends_with(char::is_whitespace)) =>
            {
                return Some(index);
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection() -> Value {
        json!({"status":"needs_review", "active_attempt":null,
            "last_attempt":"A-123", "input_snapshot":"abc"})
    }

    #[test]
    fn preserves_handwritten_yaml_comments_order_body_and_crlf() {
        let input = "---\r\n# 备注\r\nstatus: draft  # 状态\r\ncustom: {nested: [1, 2]}\r\nrole: implementer\r\nactive_attempt: \"A-old # text\" # retained\r\nlast_attempt: null\r\ninput_snapshot: null\r\n---\r\n\r\n手写正文\r\n";
        let expected = input
            .replace("status: draft", "status: \"needs_review\"")
            .replace("\"A-old # text\"", "null")
            .replace("last_attempt: null", "last_attempt: \"A-123\"")
            .replace("input_snapshot: null", "input_snapshot: \"abc\"");
        assert_eq!(
            apply_projection(input.as_bytes(), &projection()).unwrap(),
            expected.as_bytes()
        );
    }

    #[test]
    fn supports_quoted_alias_and_adds_only_missing_projection_fields() {
        let input = b"---\n'\xe7\x8a\xb6\xe6\x80\x81': draft # note\nrequested_action: cancel\n---\nmanual\n";
        let result = String::from_utf8(apply_projection(input, &projection()).unwrap()).unwrap();
        assert!(result.contains("'状态': \"needs_review\" # note\n"));
        assert!(result.contains("requested_action: cancel\n"));
        assert_eq!(result.matches("active_attempt:").count(), 1);
        assert!(result.ends_with("---\nmanual\n"));
    }

    #[test]
    fn ambiguous_or_multiline_projection_fields_fail_without_rewriting() {
        for input in [
            "---\nstatus: draft\n状态: running\n---\nbody",
            "---\nstatus: |\n  draft\n---\nbody",
            "---\n{status: draft}\n---\nbody",
            "---\nstatus: [draft]\n---\nbody",
            "---\nstatus:\n  draft\n---\nbody",
        ] {
            assert!(
                apply_projection(input.as_bytes(), &projection()).is_err(),
                "accepted {input:?}"
            );
        }
    }

    #[test]
    fn projection_is_idempotent_and_does_not_interpret_requested_action() {
        let input = b"---\nrequested_action: run\n---\n# task\n";
        let once = apply_projection(input, &projection()).unwrap();
        assert_eq!(apply_projection(&once, &projection()).unwrap(), once);
        assert!(
            String::from_utf8(once)
                .unwrap()
                .contains("requested_action: run")
        );
    }

    #[test]
    fn anchored_projection_is_rejected_without_breaking_unknown_aliases() {
        let input = b"---\nstatus: &state draft\ncustom: *state\n---\nbody\n";
        assert!(apply_projection(input, &projection()).is_err());
    }

    #[test]
    fn apostrophe_in_plain_scalar_does_not_swallow_its_comment() {
        let input = b"---\nstatus: someone's draft # retained\n---\nbody\n";
        let result = String::from_utf8(apply_projection(input, &projection()).unwrap()).unwrap();
        assert!(result.contains("status: \"needs_review\" # retained\n"));
    }
}
