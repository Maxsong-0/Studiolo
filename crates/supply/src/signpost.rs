//! 冷启动路牌：六项全部从白名单内的结构化 Markdown 派生，不读任何人写的「路牌文件」。
//!
//! 只给路牌不给正文（ADR-015 二）。超过内部目标时按固定优先级逐级裁剪，最后由硬预算兜底：
//! 超出 2000 token 即失败，不是告警。

mod render;
mod source;

use render::render;
use source::{project_status, rules_summary};

use studiolo_core::VaultPath;

use crate::{BudgetExceeded, HARD_BUDGET_TOKENS, Snapshot, TARGET_BUDGET_TOKENS, check_budget};

/// 接入方身份（类型 ＋ 实例）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    /// 类型名（claude / codex / grok …）。
    pub agent_type: String,
    /// 实例 id（ULID）。
    pub instance_id: String,
    /// 实例注册时间（RFC 3339）。
    pub registered_at: String,
}

/// 路牌派生输入。
pub struct SignpostInput<'a> {
    /// 当前快照。
    pub snapshot: &'a Snapshot,
    /// 白名单前缀（配置顺序）。
    pub allow: &'a [VaultPath],
    /// 接入方身份。
    pub principal: &'a Principal,
    /// 快照代次。
    pub generation: u64,
    /// 生成时间（RFC 3339）。
    pub generated_at: &'a str,
    /// 与当前副本绑定的新鲜度说明，计入所有裁剪等级的硬预算。
    pub freshness: Option<&'a str>,
    /// 本外壳暴露的 tool 名与一句说明。
    pub tools: &'a [(&'a str, &'a str)],
}

/// 派生结果：文本、计量与用于审计的摘要。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signpost {
    /// 路牌全文（Markdown）。
    pub text: String,
    /// o200k 计量。
    pub tokens: usize,
    /// 生效的裁剪等级（0 = 未裁剪）。
    pub trim_level: u8,
}

/// 一个白名单项目的派生状态；缺失字段显示「不可用」。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectStatus {
    prefix: String,
    fields: Vec<(String, String)>,
    focus: Option<String>,
    in_progress: Vec<String>,
    unclaimed: Vec<String>,
    entries: Vec<(String, String)>,
}

const STATUS_KEYS: [&str; 4] = ["当前版本", "状态", "最近日志", "未关闭 BUG"];

/// 派生并计量路牌；超过硬预算返回错误（二元验收）。
pub fn derive(input: &SignpostInput<'_>) -> Result<Signpost, BudgetExceeded> {
    let projects = input
        .allow
        .iter()
        .filter_map(|prefix| project_status(input.snapshot, prefix))
        .collect::<Vec<_>>();
    let rules = rules_summary(input.snapshot, input.allow);
    let mut last = None;
    for level in 0..=3u8 {
        let text = render(input, &projects, &rules, level);
        match check_budget(&text) {
            Ok(tokens) if tokens <= TARGET_BUDGET_TOKENS => {
                return Ok(Signpost {
                    text,
                    tokens,
                    trim_level: level,
                });
            }
            Ok(tokens) => last = Some((text, tokens, level)),
            Err(error) => {
                if level == 3 {
                    return Err(error);
                }
            }
        }
    }
    // 三级裁剪后仍在目标之上但未超硬预算：接受，并记录裁剪等级。
    let (text, tokens, level) = last.ok_or(BudgetExceeded {
        tokens: usize::MAX,
        limit: HARD_BUDGET_TOKENS,
    })?;
    Ok(Signpost {
        text,
        tokens,
        trim_level: level,
    })
}
