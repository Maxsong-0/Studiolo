//! 逐行归属（T-210）：从当前正文沿可查历史往回走，一行归它第一次出现的那一版。
//!
//! 不猜：当前正文不在历史里的那部分标「没有记录」，一直留到最早一版的行标「早于可查历史」，
//! 只有那一版确实是创建时才归给它的主体；版本的主体未知就照实给 null。
//! 这里只有纯逻辑和 JSON 投影，读 Broker 的事在 `coordinator/history.rs`。
#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use std::collections::HashMap;
use studiolo_core::{Revision, VaultPath, VaultUuid};
use studiolo_vault_broker::HistoryLineage;

/// 最多回看多少版；和历史保留策略「最近 50 版」对齐。
pub(crate) const MAX_VERSIONS: usize = 50;
/// 当前正文最多多少行。
pub(crate) const MAX_LINES: usize = 20_000;
/// 一次归属最多做多少步比较（行访问加表格单元）；超出就不提供，不给半截结果。
const WORK_BUDGET: usize = 20_000_000;
/// 没有锚点的一小段用最长公共子序列，表格最多这么多格（u16，约 0.5 MiB）。
const LCS_CELLS: usize = 512 * 512;

/// 一行的归属。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// 第一次出现在版本链的第 n 版（0 是最新）。
    Version(usize),
    /// 当前正文不在历史里（库外直接改的文件），这一行是那次改动带来的。
    Untracked,
    /// 一直存在到可查历史的最早一版，那一版又不是创建：更早是谁写的已经查不到。
    BeyondHistory,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Failure<E> {
    /// 行数或比较量超出上限。
    TooLarge,
    /// 读某一版正文失败。
    Read(E),
}

fn spend<E>(budget: &mut usize, cost: usize) -> Result<(), Failure<E>> {
    *budget = budget.checked_sub(cost).ok_or(Failure::TooLarge)?;
    Ok(())
}

/// 两边都只出现一次的相同行里，次序一致的最长一组；返回 (a 下标, b 下标)，两列都严格递增。
fn anchors(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let mut seen: HashMap<&str, (u32, usize, u32, usize)> = HashMap::new();
    for (i, line) in a.iter().enumerate() {
        let entry = seen.entry(*line).or_insert((0, 0, 0, 0));
        entry.0 += 1;
        entry.1 = i;
    }
    for (j, line) in b.iter().enumerate() {
        if let Some(entry) = seen.get_mut(*line) {
            entry.2 += 1;
            entry.3 = j;
        }
    }
    let mut pairs: Vec<(usize, usize)> = seen
        .values()
        .filter(|entry| entry.0 == 1 && entry.2 == 1)
        .map(|entry| (entry.1, entry.3))
        .collect();
    pairs.sort_unstable();
    // 耐心排序求最长递增子序列：tails[k] 是长度 k+1 的递增组里结尾最小的那一个。
    let mut tails: Vec<usize> = Vec::new();
    let mut previous: Vec<Option<usize>> = vec![None; pairs.len()];
    for (index, &(_, j)) in pairs.iter().enumerate() {
        let position = tails.partition_point(|&tail| pairs[tail].1 < j);
        if position > 0 {
            previous[index] = Some(tails[position - 1]);
        }
        if position == tails.len() {
            tails.push(index);
        } else {
            tails[position] = index;
        }
    }
    let mut run = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied();
    while let Some(index) = at {
        run.push(pairs[index]);
        at = previous[index];
    }
    run.reverse();
    run
}

/// 小段的最长公共子序列；调用方保证格数不超过 [`LCS_CELLS`]，所以长度装得进 u16。
fn lcs(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let width = m + 1;
    let mut table = vec![0u16; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if a[i] == b[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut pairs = Vec::new();
    while i < n && j < m {
        if a[i] == b[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// 给 `newer` 的每一行找它在 `older` 里对应的行；找不到的就是这一版新写的。
///
/// 去掉相同的头尾，中间用锚点切开再逐段做；没有锚点的小段用最长公共子序列，大段整段算新写。
/// 任何一组保持先后次序的相同行都是合法答案，所以算法只影响「认出多少行没动」，
/// 不会把内容不同的两行说成同一行。
fn carry<E>(
    newer: &[&str],
    older: &[&str],
    budget: &mut usize,
) -> Result<Vec<Option<usize>>, Failure<E>> {
    let mut out = vec![None; newer.len()];
    let mut stack = vec![(0, newer.len(), 0, older.len())];
    while let Some((mut a0, mut a1, mut b0, mut b1)) = stack.pop() {
        while a0 < a1 && b0 < b1 && newer[a0] == older[b0] {
            out[a0] = Some(b0);
            a0 += 1;
            b0 += 1;
        }
        while a0 < a1 && b0 < b1 && newer[a1 - 1] == older[b1 - 1] {
            a1 -= 1;
            b1 -= 1;
            out[a1] = Some(b1);
        }
        if a0 == a1 || b0 == b1 {
            continue;
        }
        spend(budget, (a1 - a0) + (b1 - b0))?;
        let found = anchors(&newer[a0..a1], &older[b0..b1]);
        if found.is_empty() {
            let cells = (a1 - a0 + 1) * (b1 - b0 + 1);
            if cells <= LCS_CELLS {
                spend(budget, cells)?;
                for (i, j) in lcs(&newer[a0..a1], &older[b0..b1]) {
                    out[a0 + i] = Some(b0 + j);
                }
            }
            continue;
        }
        let (mut from_a, mut from_b) = (a0, b0);
        for (i, j) in found {
            out[a0 + i] = Some(b0 + j);
            stack.push((from_a, a0 + i, from_b, b0 + j));
            from_a = a0 + i + 1;
            from_b = b0 + j + 1;
        }
        stack.push((from_a, a1, from_b, b1));
    }
    Ok(out)
}

/// 算每一行的归属。
///
/// - `current_is_newest`：当前正文就是版本链第 0 版（revision 相同）；
/// - `versions`：版本链长度；`created(n)`：第 n 版是不是创建；`read(n)`：第 n 版正文。
///
/// 一次只留两版正文在内存里。
pub(crate) fn attribute<E>(
    current: &str,
    current_is_newest: bool,
    versions: usize,
    created: impl Fn(usize) -> bool,
    mut read: impl FnMut(usize) -> Result<String, E>,
) -> Result<Vec<Origin>, Failure<E>> {
    let total = current.lines().count();
    if total > MAX_LINES {
        return Err(Failure::TooLarge);
    }
    let mut origins = vec![Origin::BeyondHistory; total];
    // 还没定下归属的行：(当前正文里的行号, 它在「上面这一版」里的行号)。
    let mut alive: Vec<(usize, usize)> = (0..total).map(|line| (line, line)).collect();
    let mut top = current.to_owned();
    // 上面这一版是版本链的第几版；None 表示它是不在历史里的当前正文。
    let mut top_version = (versions > 0 && current_is_newest).then_some(0);
    let mut next = usize::from(top_version.is_some());
    let mut budget = WORK_BUDGET;
    while !alive.is_empty() {
        if let Some(version) = top_version
            && created(version)
        {
            for (line, _) in alive.drain(..) {
                origins[line] = Origin::Version(version);
            }
            break;
        }
        if next >= versions {
            break;
        }
        let older = read(next).map_err(Failure::Read)?;
        let matched = {
            let newer_lines: Vec<&str> = top.lines().collect();
            let older_lines: Vec<&str> = older.lines().collect();
            if older_lines.len() > MAX_LINES {
                return Err(Failure::TooLarge);
            }
            carry(&newer_lines, &older_lines, &mut budget)?
        };
        let mut kept = Vec::with_capacity(alive.len());
        for (line, at) in alive {
            match matched[at] {
                Some(position) => kept.push((line, position)),
                None => {
                    origins[line] = top_version.map_or(Origin::Untracked, Origin::Version);
                }
            }
        }
        alive = kept;
        top = older;
        top_version = Some(next);
        next += 1;
    }
    Ok(origins)
}

/// 正文的普通 SHA-256（十六进制）。网页用它确认自己手里的原文和这份归属是同一版。
pub(crate) fn content_digest(content: &str) -> String {
    studiolo_core::sha256_hex(content)
}

/// 把逐行归属合并成连续的段再投影成 JSON；行号从 1 起，两端都含。
pub(crate) fn response(
    vault: VaultUuid,
    path: &VaultPath,
    revision: Revision,
    content: &str,
    lineage: &HistoryLineage,
    origins: &[Origin],
) -> Value {
    let mut runs = Vec::new();
    let mut start = 0;
    while start < origins.len() {
        let mut end = start;
        while end + 1 < origins.len() && origins[end + 1] == origins[start] {
            end += 1;
        }
        let (kind, version) = match origins[start] {
            Origin::Version(n) => ("version", Some(&lineage.entries[n].entry)),
            Origin::Untracked => ("untracked", None),
            Origin::BeyondHistory => ("beyond_history", None),
        };
        runs.push(json!({
            "from_line": start + 1,
            "to_line": end + 1,
            "origin": kind,
            "history_id": version.map(|entry| entry.id.clone()),
            "revision": version.map(|entry| entry.revision.to_hex()),
            "author": version.map_or(Value::Null, |entry| super::response::actor(entry.author.clone())),
            "authored_at_ms": version.and_then(|entry| entry.authored_at_ms),
        }));
        start = end + 1;
    }
    json!({
        "vault_uuid": vault.to_string(),
        "path": path.to_string(),
        "revision": revision.to_hex(),
        "content_sha256": content_digest(content),
        "lines": origins.len(),
        "versions": lineage.entries.len(),
        "more_history": lineage.more,
        "runs": runs,
    })
}
