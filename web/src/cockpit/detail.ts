/**
 * 选中行就地展开的细节（T-197，纯函数）：不用先跳到别的页面，就能看到这一件改了什么、占了哪里。
 * 只读：把已经在手的字段排成几段小清单，不补数据、不发请求；批准、退回仍在拥有它的页面上做。
 *
 * - 策略草案：校验问题、扩大项、收窄项、改动条目（校验报告里的四样）；还没有校验报告时不列。
 * - 黑板任务：写域（它占着的路径）。回执不列：汇总接口里只有已结束的任务才带回执，等你的任务都没有。
 * - 工作台待办：接口只给标题和出处，出处已经写在上面那一行，这里不再重复。
 *
 * 每段最多列 {@link DETAIL_LIMIT} 条，多的写明还有几条；一截文字最长 {@link DETAIL_TEXT_MAX} 个字，超了截短。
 * 就地批准的执行预览开着时（T-200）不截条数，全部列出：要点头的东西得先看全。
 */
import type { DiffEntry, Expansion } from '../policyModel';
import type { PendingEntry } from './pending';

export const DETAIL_LIMIT = 6;
export const DETAIL_TEXT_MAX = 96;

export interface DetailMark {
  /** 行首的小记号：■ 扩大、□ 收窄、＋ 新增、－ 删除、～ 改动。 */
  glyph: string;
  /** 记号的读法：记号只是辅助，读屏念这个词。 */
  name: string;
}

export interface DetailLine {
  mark: DetailMark | null;
  /** 等宽的那一截：路径、字段位置；没有时为 null。 */
  code: string | null;
  /** 后面的几截说明，各自成段（「原：…」「改为：…」）。 */
  parts: string[];
  /** 最后的灰字备注。 */
  note: string | null;
}

export interface DetailSection {
  id: 'problems' | 'expansions' | 'narrowings' | 'diff' | 'paths';
  label: string;
  /** 这一段一共几条。 */
  count: number;
  lines: DetailLine[];
  /** 没列出来的条数。 */
  more: number;
  /** 段尾的一句说明。 */
  note: string | null;
}

const EXPANSION: DetailMark = { glyph: '■', name: '扩大' };
const NARROWING: DetailMark = { glyph: '□', name: '收窄' };
const OPS: Record<DiffEntry['op'], DetailMark> = {
  added: { glyph: '＋', name: '新增' },
  removed: { glyph: '－', name: '删除' },
  changed: { glyph: '～', name: '改动' },
};

/** 一截文字：空白折成一个空格，太长就截短加省略号。按字（码点）数，不把一个字切成两半。 */
export function brief(text: string): string {
  const flat = text.replace(/\s+/gu, ' ').trim();
  const letters = Array.from(flat);
  return letters.length > DETAIL_TEXT_MAX
    ? `${letters.slice(0, DETAIL_TEXT_MAX).join('')}…`
    : flat;
}

/** 「原：…」「改为：…」两截，没有的那一边不写。 */
function change(before: string | null, after: string | null): string[] {
  return [
    ...(before !== null ? [`原：${brief(before)}`] : []),
    ...(after !== null ? [`改为：${brief(after)}`] : []),
  ];
}

const changeLine =
  (mark: DetailMark) =>
  (item: Expansion): DetailLine => ({
    mark,
    code: item.path,
    parts: change(item.before, item.after),
    note: item.note || null,
  });

function diffLine(entry: DiffEntry): DetailLine {
  return {
    mark: OPS[entry.op],
    code: entry.path,
    parts:
      entry.op === 'changed'
        ? change(entry.before, entry.after)
        : [brief(entry.after ?? entry.before ?? '')].filter(Boolean),
    note: null,
  };
}

/**
 * 一段：没有条目就不出这一段。`limit` 是最多列几条；`total` 是一共几条
 * （接口另给了总数时用它，条目本身可能被截断）。
 */
function section<Item>(
  id: DetailSection['id'],
  label: string,
  items: readonly Item[],
  toLine: (item: Item) => DetailLine,
  limit: number,
  { total = items.length, note = null as string | null } = {},
): DetailSection[] {
  if (items.length === 0) return [];
  const lines = items.slice(0, limit).map(toLine);
  const count = Math.max(total, items.length);
  return [{ id, label, count, lines, more: count - lines.length, note }];
}

/** `limit` 是每段最多列几条；就地批准的预览开着时传 `Infinity`，全部列出。 */
export function entryDetail(
  entry: PendingEntry,
  limit: number = DETAIL_LIMIT,
): DetailSection[] {
  switch (entry.source) {
    case 'policy': {
      const report = entry.desired.report;
      if (!report) return [];
      const { diff } = report;
      return [
        ...section(
          'problems',
          '问题',
          report.problems,
          (problem) => ({
            mark: null,
            code: problem.path,
            parts: [brief(problem.message)],
            note: null,
          }),
          limit,
        ),
        ...section(
          'expansions',
          '扩大',
          report.expansions,
          changeLine(EXPANSION),
          limit,
        ),
        ...section(
          'narrowings',
          '收窄',
          report.narrowings,
          changeLine(NARROWING),
          limit,
        ),
        ...section('diff', '改动', diff.entries, diffLine, limit, {
          total: diff.added + diff.removed + diff.changed,
          note: diff.truncated ? '改动太多，接口只给了前一部分。' : null,
        }),
      ];
    }
    case 'board':
      return section(
        'paths',
        '写域',
        entry.item.paths,
        (path) => ({
          mark: null,
          code: path,
          parts: [],
          note: null,
        }),
        limit,
      );
    case 'inbox':
      return [];
  }
}
