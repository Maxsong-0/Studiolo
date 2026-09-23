//! 任务页本身：校验 frontmatter 身份（type 与 id），并为新任务渲染初始页面。

use serde_json::{Value, json};
use studiolo_core::TaskId;

use crate::TaskSpec;

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
