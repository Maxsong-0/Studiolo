//! 冷启动路牌：六项全部从白名单内的结构化 Markdown 派生，不读任何人写的「路牌文件」。
//!
//! 只给路牌不给正文（ADR-015 二）。超过内部目标时按固定优先级逐级裁剪，最后由硬预算兜底：
//! 超出 2000 token 即失败，不是告警。

mod render;
mod source;

use render::render;
use source::{principle_titles, project_status, rules_summary};

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
    for level in 0..=3u8 {
        let text = render(input, &projects, &rules, level, entry_label);
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
