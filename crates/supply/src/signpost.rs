//! 冷启动路牌：六项全部从白名单内的结构化 Markdown 派生，不读任何人写的「路牌文件」。
//!
//! 只给路牌不给正文（ADR-015 二）。超过内部目标时按固定优先级逐级裁剪，最后由硬预算兜底：
//! 超出硬预算（`HARD_BUDGET_TOKENS`）即失败，不是告警。另有一条可见段上限（`CLIENT_VISIBLE_UTF16`）：
//! 要紧的节排在前面，放不下时只裁这一段，不失败（vault BUG-061）。

mod render;
mod source;

use render::{Rendered, Trim, render};
use source::{RulesSummary, principle_titles, project_status, rules_summary};

use studiolo_core::VaultPath;

use crate::{
    BudgetExceeded, CLIENT_VISIBLE_UTF16, HARD_BUDGET_TOKENS, Snapshot, TARGET_BUDGET_TOKENS,
    check_budget, count_tokens,
};

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

/// 策略平面接入后，路牌第 3／5／6 项改从 effective 派生的输入（M4）。
///
/// 没有策略平面（`serve` 未给 `--policy`，M3 兼容模式）时为 `None`，路牌明写「策略平面：未接入」。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PolicySignpost {
    /// 当前入口具有实时黑板写能力（HTTP兜底固定false）。
    pub board_write: bool,
    /// 复核时的 effective 哈希。
    pub effective_hash: String,
    /// 该类型现行能力集版本。
    pub capability_version: u64,
    /// 供给视图（部署上限 ∩ 能力集白名单）。
    pub read_prefixes: Vec<VaultPath>,
    /// 能力集是否允许以 patch 形式写正文（M5 起才有落地面）。
    pub content_patch_only: bool,
    /// 能力集是否允许提交 desired 提案。
    pub propose_desired: bool,
    /// 硬绑定（触发条件 → 必须叠加的 skill），策略强制。
    pub hard_bindings: Vec<(String, String)>,
    /// 现行规则母本。
    pub rules: Vec<PolicyRule>,
    /// 现行 skill 名。
    pub skills: Vec<String>,
}

/// 一份现行规则母本。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PolicyRule {
    /// subject（文件名）。
    pub subject: String,
    /// 版本号。
    pub version: u64,
    /// 正文（Markdown）。
    pub body: String,
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
    /// 副本快到协议上限的提醒（`replica::capacity_notice_for`），写在第 2 节开头、各裁剪等级都保留。
    /// 不放在新鲜度后面：部署验收和客户端投影按固定行号找身份段（第 3 或第 4 行），多一行就判路牌无效。
    pub capacity: Option<&'a str>,
    /// 本外壳暴露的 tool 名与一句说明。
    pub tools: &'a [(&'a str, &'a str)],
    /// 策略平面派生项；`None` ＝ 未接入。
    pub policy: Option<&'a PolicySignpost>,
    /// 规则名单；None＝不过滤（未接策略平面）。
    pub rules_whitelist: Option<&'a [String]>,
}

/// 派生结果：文本、计量与用于审计的摘要。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signpost {
    /// 路牌全文（Markdown）。
    pub text: String,
    /// o200k 计量。
    pub tokens: usize,
    /// 生效的最深裁剪等级（0 = 未裁剪）；可见段会比项目清单裁得深。
    pub trim_level: u8,
    /// 黑板行的字节位置，见 [`Signpost::insert_board`]。
    board_at: usize,
}

impl Signpost {
    /// 把黑板行放到第 4 节标题下一行并重新计量；`line` 自带结尾换行，只调一次。
    ///
    /// 放在这一节最前面：派生时只给它留了 [`BOARD_LINE_RESERVE_UTF16`]，更长时被挤出可见段的是待办尾部。
    pub fn insert_board(&mut self, line: &str) {
        self.text.insert_str(self.board_at, line);
        self.tokens = count_tokens(&self.text);
    }
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

/// 给黑板行留的可见段余量：没有子项时 28 个单元，子项多到改报条数时约 50 个。
const BOARD_LINE_RESERVE_UTF16: usize = 64;
/// 最深的裁剪等级。
const MAX_TRIM: u8 = 3;

const STATUS_KEYS: [&str; 4] = ["当前版本", "状态", "最近日志", "未关闭 BUG"];

/// 派生并计量路牌；超过硬预算返回错误（二元验收）。
pub fn derive(input: &SignpostInput<'_>) -> Result<Signpost, BudgetExceeded> {
    derive_with_label(input, "tools")
}

/// HTTP 外壳复用同一路牌、裁剪与硬预算，续读部分明确标记 HTTP 入口。
pub fn derive_http(input: &SignpostInput<'_>) -> Result<Signpost, BudgetExceeded> {
    derive_with_label(input, "HTTP 入口")
}

fn derive_with_label(
    input: &SignpostInput<'_>,
    entry_label: &str,
) -> Result<Signpost, BudgetExceeded> {
    let projects = input
        .allow
        .iter()
        .filter_map(|prefix| project_status(input.snapshot, prefix))
        .collect::<Vec<_>>();
    let mut rules = rules_summary(input.snapshot, input.allow, input.rules_whitelist);
    if let Some(policy) = input.policy {
        // 策略平面的规则母本优先于 vault 里的文件：effective 才是权威，文件只是 desired 文本。
        if let Some(principles) = policy
            .rules
            .iter()
            .map(|rule| principle_titles(&rule.body))
            .find(|titles| !titles.is_empty())
        {
            rules.principles = principles;
        }
        rules.enforced = policy
            .hard_bindings
            .iter()
            .map(|(when, skill)| format!("{when} → {skill}"))
            .collect();
        if !rules.enforced.is_empty() {
            rules.hard_bindings.clear();
        }
        for rule in &policy.rules {
            rules.entries.push((
                format!("policy://rules/{}", rule.subject),
                format!("v{}", rule.version),
            ));
        }
    }
    let mut last = None;
    for level in 0..=MAX_TRIM {
        let (rendered, trim_level) =
            render_within_view(input, &projects, &rules, level, entry_label);
        match check_budget(&rendered.text) {
            Ok(tokens) => {
                let signpost = Signpost {
                    text: rendered.text,
                    tokens,
                    trim_level,
                    board_at: rendered.board_at,
                };
                if tokens <= TARGET_BUDGET_TOKENS {
                    return Ok(signpost);
                }
                last = Some(signpost);
            }
            Err(error) => {
                if level == MAX_TRIM {
                    return Err(error);
                }
            }
        }
    }
    // 最深一级仍在目标之上但未超硬预算：接受，并记录裁剪等级。
    last.ok_or(BudgetExceeded {
        tokens: usize::MAX,
        limit: HARD_BUDGET_TOKENS,
    })
}

/// 项目清单按 `level` 裁；可见段从 `level` 起往深裁到客户端看得全为止，返回实际用的可见段等级。
///
/// 最深一级仍放不下就照发：硬约束和续读入口排在最前，被截的只会是它们后面的节。
fn render_within_view(
    input: &SignpostInput<'_>,
    projects: &[ProjectStatus],
    rules: &RulesSummary,
    level: u8,
    entry_label: &str,
) -> (Rendered, u8) {
    let mut front = level;
    loop {
        let trim = Trim {
            front,
            projects: level,
        };
        let rendered = render(input, projects, rules, trim, entry_label);
        let fits = rendered.front_utf16 + BOARD_LINE_RESERVE_UTF16 <= CLIENT_VISIBLE_UTF16;
        if fits || front == MAX_TRIM {
            return (rendered, front);
        }
        front += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 可见段至少裁到 token 预算定的那一级：看得全不等于可以少裁。
    #[test]
    fn the_visible_part_is_never_trimmed_less_than_the_token_budget_asks() {
        let snapshot = Snapshot::default();
        let principal = Principal {
            agent_type: "claude".into(),
            instance_id: "01JT3STUDIOLO0000000000001".into(),
            registered_at: "2026-11-28T01:00:00Z".into(),
        };
        let input = SignpostInput {
            snapshot: &snapshot,
            allow: &[],
            principal: &principal,
            generation: 1,
            generated_at: "2026-11-28T01:00:00Z",
            freshness: None,
            capacity: None,
            tools: &[("handshake", "重取路牌")],
            policy: None,
            rules_whitelist: None,
        };
        for level in 0..=MAX_TRIM {
            let (_, front) =
                render_within_view(&input, &[], &RulesSummary::default(), level, "tools");
            assert_eq!(front, level);
        }
    }
}
