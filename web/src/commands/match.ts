/**
 * ⌘K 面板的本地匹配：输入按空白切成若干片段，每段都要在名字、说明或别名里出现（不区分大小写）；
 * 名字里命中的片段标出来，面板用红色下划线画（设计规范第 8 节「匹配字下划红线」）。
 */

/** 名字里要标红线的一段，[起, 止) 按 UTF-16 下标。 */
export type MatchRange = readonly [start: number, end: number];

export interface CommandMatch {
  /** 0＝名字以整句开头，1＝每段都在名字里，2＝有的段只在说明或别名里；小的排前面。 */
  score: 0 | 1 | 2;
  ranges: MatchRange[];
}

export interface Matchable {
  label: string;
  hint?: string;
  keywords: readonly string[];
}

/** 输入切成小写片段；只有空白时返回空数组（此时全部命令都算匹配）。 */
export function queryTerms(query: string): string[] {
  return query
    .trim()
    .toLocaleLowerCase()
    .split(/\s+/u)
    .filter((term) => term.length > 0);
}

/**
 * 每段在文字里第一次出现的位置，重叠的合并。
 * 小写化改变了长度（极少数字符）时下标对不上原文，宁可不标也不标错。
 */
export function highlightRanges(
  text: string,
  terms: readonly string[],
): MatchRange[] {
  const lower = text.toLocaleLowerCase();
  if (lower.length !== text.length) return [];
  const found = terms
    .map((term): MatchRange | null => {
      const at = lower.indexOf(term);
      return at < 0 ? null : [at, at + term.length];
    })
    .filter((range): range is MatchRange => range !== null)
    .sort((left, right) => left[0] - right[0]);
  const merged: [number, number][] = [];
  for (const [start, end] of found) {
    const last = merged.at(-1);
    if (last && start <= last[1]) last[1] = Math.max(last[1], end);
    else merged.push([start, end]);
  }
  return merged;
}

/** 不匹配返回 null；片段为空时一律匹配、不标线。 */
export function matchCommand(
  command: Matchable,
  terms: readonly string[],
): CommandMatch | null {
  if (!terms.length) return { score: 0, ranges: [] };
  const label = command.label.toLocaleLowerCase();
  const haystack = [label, command.hint ?? '', ...command.keywords]
    .join('\n')
    .toLocaleLowerCase();
  if (!terms.every((term) => haystack.includes(term))) return null;
  const score = label.startsWith(terms.join(' '))
    ? 0
    : terms.every((term) => label.includes(term))
      ? 1
      : 2;
  return { score, ranges: highlightRanges(command.label, terms) };
}
