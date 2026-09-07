//! 冷启动路牌的 token 预算：o200k_base 计量，≤2000 为硬上限（二元验收），内部目标 ≤1500。
//!
//! 选 o200k 作为公开、可复现的计量基准；Claude 的分词器不公开，差异由 500 token 的余量吸收。

use std::sync::OnceLock;

use tiktoken_rs::CoreBPE;

/// 路牌硬上限：超出即失败，不是告警（ADR-015 二）。
pub const HARD_BUDGET_TOKENS: usize = 2000;
/// 派生器的内部目标；超过它先按优先级裁剪，再由硬上限兜底。
pub const TARGET_BUDGET_TOKENS: usize = 1500;

/// 路牌超出硬预算。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("路牌 {tokens} token 超过硬预算 {limit}")]
pub struct BudgetExceeded {
    /// 实际计量结果。
    pub tokens: usize,
    /// 生效的上限。
    pub limit: usize,
}

fn encoder() -> &'static CoreBPE {
    static ENCODER: OnceLock<CoreBPE> = OnceLock::new();
    ENCODER.get_or_init(|| {
        tiktoken_rs::o200k_base().expect("o200k_base 词表内嵌于二进制，构建不可能失败")
    })
}

/// 用 o200k_base 计量文本（不计特殊 token）。
#[must_use]
pub fn count_tokens(text: &str) -> usize {
    encoder().encode_ordinary(text).len()
}

/// 计量并按硬上限判定；返回实际 token 数。
pub fn check_budget(text: &str) -> Result<usize, BudgetExceeded> {
    let tokens = count_tokens(text);
    if tokens > HARD_BUDGET_TOKENS {
        return Err(BudgetExceeded {
            tokens,
            limit: HARD_BUDGET_TOKENS,
        });
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_deterministic_and_zero_for_empty_text() {
        assert_eq!(count_tokens(""), 0);
        let sample = "Studiolo：一个 URL 让任何 AI 立刻知道自己在干什么。";
        let first = count_tokens(sample);
        assert!(first > 5 && first < 60, "unexpected count {first}");
        assert_eq!(first, count_tokens(sample));
    }

    #[test]
    fn hard_budget_is_binary() {
        let unit = "当前重点：M3 首片冷启动路牌与白名单供给。\n";
        let mut text = String::new();
        while count_tokens(&text) <= HARD_BUDGET_TOKENS {
            text.push_str(unit);
        }
        let error = check_budget(&text).unwrap_err();
        assert!(error.tokens > HARD_BUDGET_TOKENS);
        assert_eq!(error.limit, HARD_BUDGET_TOKENS);
        let within = unit.repeat(10);
        assert!(check_budget(&within).unwrap() <= TARGET_BUDGET_TOKENS);
    }
}
