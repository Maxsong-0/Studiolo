//! 按 restic 0.19.1 `check --json` 的固定输出归类。
//!
//! 这个版本在 `--json` 下每次都往 stdout 打一行 summary（`num_errors`、`broken_packs`），打不开仓库、
//! 连不上、凭据不对时也打，且出错时 `num_errors` 至少为 1；错误明细走 stderr。所以有 summary 不代表
//! 检查跑完了：只有退出 0、恰好一个 summary、没有错误也没有损坏的数据包才算干净。
use super::{CheckOutcome, IncompleteReason};
use serde_json::Value;

/// `exit_success` 是退出码是不是 0，`stdout` 是完整的标准输出。
#[must_use]
pub fn classify_check_output(exit_success: bool, stdout: &[u8]) -> CheckOutcome {
    let Some((errors, broken_packs)) = summary(stdout) else {
        return CheckOutcome::incomplete(IncompleteReason::Protocol);
    };
    match CheckOutcome::problem(errors, broken_packs) {
        Some(problem) => problem,
        None if exit_success => CheckOutcome::clean(),
        // 退出码不是 0 却一个错误都没报：和固定版本的行为对不上，不当结论。
        None => CheckOutcome::incomplete(IncompleteReason::Protocol),
    }
}

/// 唯一那个 summary 里的错误数和损坏数据包个数；任何一行不是 JSON 对象、没有或不止一个 summary、
/// 字段类型不对，都是 `None`。
fn summary(stdout: &[u8]) -> Option<(u64, u64)> {
    let mut found = None;
    for line in stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: Value = serde_json::from_slice(line).ok()?;
        let object = value.as_object()?;
        if object.get("message_type").and_then(Value::as_str) != Some("summary") {
            continue;
        }
        if found.is_some() {
            return None;
        }
        let errors = object.get("num_errors")?.as_u64()?;
        let broken_packs = match object.get("broken_packs")? {
            Value::Null => 0,
            Value::Array(packs) if packs.iter().all(Value::is_string) => {
                u64::try_from(packs.len()).ok()?
            }
            _ => return None,
        };
        found = Some((errors, broken_packs));
    }
    found
}
