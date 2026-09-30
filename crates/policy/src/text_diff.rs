//! 先裁共同首尾，再对变化区间计算有界 LCS；完整删除事件不受展示条数限制。

use super::{Diff, DiffEntry, DiffOp, MAX_ENTRIES};

const MAX_LCS_CELLS: usize = 4_000_000;

pub(crate) fn text_diff(before: &str, after: &str) -> Diff {
    text_diff_with_removals(before, after, |_, _| {})
}

/// 删除回调覆盖完整差异，供规则扩大项判定；展示截断不能截断校验。
pub(crate) fn text_diff_with_removals(
    before: &str,
    after: &str,
    mut removed: impl FnMut(usize, &str),
) -> Diff {
    let mut diff = Diff::empty("text");
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old = &old[prefix..old.len() - suffix];
    let new = &new[prefix..new.len() - suffix];
    // 空侧不分配 LCS；预算包含边界行列，避免窄长输入绕过内存上限。
    let cells = (old.len() + 1).saturating_mul(new.len() + 1);
    if old.is_empty() || new.is_empty() || cells > MAX_LCS_CELLS {
        diff.truncated = !old.is_empty() && !new.is_empty() && cells > MAX_LCS_CELLS;
        for (index, line) in old.iter().enumerate() {
            let line_number = prefix + index + 1;
            removed(line_number, line);
            emit(&mut diff, DiffOp::Removed, line_number, line);
        }
        for (index, line) in new.iter().enumerate() {
            emit(&mut diff, DiffOp::Added, prefix + index + 1, line);
        }
        return diff;
    }
    // 连续分配，避免每行一个堆分配；相同首尾不参与二次复杂度的动态规划。
    let width = new.len() + 1;
    let mut table = vec![0u32; cells];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[i * width + j] = if old[i] == new[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            i += 1;
            j += 1;
        } else if j < new.len()
            && (i == old.len() || table[i * width + j + 1] >= table[(i + 1) * width + j])
        {
            emit(&mut diff, DiffOp::Added, prefix + j + 1, new[j]);
            j += 1;
        } else {
            let line_number = prefix + i + 1;
            removed(line_number, old[i]);
            emit(&mut diff, DiffOp::Removed, line_number, old[i]);
            i += 1;
        }
    }
    diff
}

fn emit(diff: &mut Diff, op: DiffOp, number: usize, line: &str) {
    // 超出展示预算仍准确计数，但不创建随后立即丢弃的正文副本。
    if diff.entries.len() == MAX_ENTRIES {
        match op {
            DiffOp::Added => diff.added += 1,
            DiffOp::Removed => diff.removed += 1,
            DiffOp::Changed => diff.changed += 1,
        }
        diff.truncated = true;
        return;
    }
    let removing = op == DiffOp::Removed;
    diff.push(DiffEntry {
        op,
        path: format!("{}:{number}", if removing { "old" } else { "new" }),
        before: removing.then(|| line.to_owned()),
        after: (!removing).then(|| line.to_owned()),
    });
}
