import { describe, expect, it } from 'vitest';
import { TASK } from '../../../dev/fixtures/ids';
import {
  DETAIL_LIMIT,
  DETAIL_TEXT_MAX,
  brief,
  entryDetail,
} from '../../cockpit/detail';
import type { OverviewState } from '../../cockpit/model';
import { planPending, type PendingEntry } from '../../cockpit/pending';
import type { ListedDesired, Report } from '../../policyModel';
import { fixtureData, policyReady } from './cockpitFixtures';

const data = fixtureData();
const ready: OverviewState = { kind: 'ready', overview: data.overview };
const [proposal] = data.desired;
const { entries } = planPending({
  board: ready,
  policy: policyReady(data.desired),
  inbox: data.workbench.inbox,
  projects: data.workbench.projects,
});
const board = (id: string) =>
  entries.find((entry) => entry.source === 'board' && entry.item.id === id)!;

const policyEntry = (desired: ListedDesired): PendingEntry => ({
  source: 'policy',
  n: 1,
  desired,
  kind: 'AI 提案',
  since: null,
  waitMs: null,
});

const withReport = (patch: Partial<Report>): PendingEntry =>
  policyEntry({ ...proposal, report: { ...proposal.report!, ...patch } });

describe('一截文字', () => {
  it('空白折成一个空格，两头去掉', () => {
    expect(brief('  第一行\n\n  第二行\t结尾  ')).toBe('第一行 第二行 结尾');
  });

  it(`超过 ${DETAIL_TEXT_MAX} 个字就截短加省略号；按字数，不把一个字切成两半`, () => {
    const exact = '字'.repeat(DETAIL_TEXT_MAX);
    expect(brief(exact)).toBe(exact);
    expect(brief(`${exact}多`)).toBe(`${exact}…`);
    const emoji = '🧭'.repeat(DETAIL_TEXT_MAX + 3);
    expect(Array.from(brief(emoji))).toHaveLength(DETAIL_TEXT_MAX + 1);
    expect(brief(emoji).endsWith('🧭…')).toBe(true);
  });
});

describe('就地细节：策略草案', () => {
  it('假数据那份提案：报告里有哪几样就列哪几段；扩大项逐条列出（方块加「扩大」）', () => {
    const report = proposal.report!;
    const sections = entryDetail(policyEntry(proposal));
    expect(sections.map((section) => section.id)).toEqual([
      ...(report.problems.length > 0 ? ['problems'] : []),
      ...(report.expansions.length > 0 ? ['expansions'] : []),
      ...(report.narrowings.length > 0 ? ['narrowings'] : []),
      ...(report.diff.entries.length > 0 ? ['diff'] : []),
    ]);
    // 假数据这份提案扩大了三项读取范围，差异里也就有条目。
    const expansions = sections.find(({ id }) => id === 'expansions')!;
    expect(expansions.label).toBe('扩大');
    expect(expansions.count).toBe(3);
    expect(expansions.more).toBe(0);
    expect(expansions.lines).toEqual(
      report.expansions.map((item) => ({
        mark: { glyph: '■', name: '扩大' },
        code: item.path,
        parts: [
          ...(item.before !== null ? [`原：${brief(item.before)}`] : []),
          ...(item.after !== null ? [`改为：${brief(item.after)}`] : []),
        ],
        note: item.note,
      })),
    );
    const diff = sections.find(({ id }) => id === 'diff')!;
    const total = report.diff.added + report.diff.removed + report.diff.changed;
    expect(diff.label).toBe('改动');
    expect(diff.count).toBe(total);
    expect(diff.lines).toHaveLength(Math.min(total, DETAIL_LIMIT));
    expect(diff.more).toBe(total - diff.lines.length);
    expect(diff.note).toBeNull();
    expect(diff.lines[0].code).toBe(report.diff.entries[0].path);
  });

  it('四样各一段，顺序固定：问题、扩大、收窄、改动', () => {
    const sections = entryDetail(
      withReport({
        ok: false,
        problems: [{ path: 'read.prefixes[0]', message: '前缀不存在' }],
        expansions: [
          { path: 'board.write', before: 'false', after: 'true', note: '多了' },
        ],
        narrowings: [
          { path: 'patch', before: 'true', after: 'false', note: '能力收回' },
        ],
        diff: {
          format: 'json',
          entries: [
            { op: 'added', path: 'a', before: null, after: '"新"' },
            { op: 'removed', path: 'b', before: '"旧"', after: null },
            { op: 'changed', path: 'c', before: '1', after: '2' },
          ],
          added: 1,
          removed: 1,
          changed: 1,
          truncated: false,
        },
      }),
    );
    expect(sections.map(({ id, label, count }) => [id, label, count])).toEqual([
      ['problems', '问题', 1],
      ['expansions', '扩大', 1],
      ['narrowings', '收窄', 1],
      ['diff', '改动', 3],
    ]);
    expect(sections[0].lines).toEqual([
      {
        mark: null,
        code: 'read.prefixes[0]',
        parts: ['前缀不存在'],
        note: null,
      },
    ]);
    expect(sections[2].lines).toEqual([
      {
        mark: { glyph: '□', name: '收窄' },
        code: 'patch',
        parts: ['原：true', '改为：false'],
        note: '能力收回',
      },
    ]);
    expect(sections[3].lines).toEqual([
      {
        mark: { glyph: '＋', name: '新增' },
        code: 'a',
        parts: ['"新"'],
        note: null,
      },
      {
        mark: { glyph: '－', name: '删除' },
        code: 'b',
        parts: ['"旧"'],
        note: null,
      },
      {
        mark: { glyph: '～', name: '改动' },
        code: 'c',
        parts: ['原：1', '改为：2'],
        note: null,
      },
    ]);
  });

  it(`每段最多列 ${DETAIL_LIMIT} 条，多的写明还有几条；接口截断了的差异按总数算并说明`, () => {
    const many = Array.from({ length: DETAIL_LIMIT + 3 }, (_, index) => ({
      path: `p${index}`,
      before: null,
      after: 'x',
      note: '多了',
    }));
    const sections = entryDetail(
      withReport({
        problems: [],
        expansions: many,
        narrowings: [],
        diff: {
          format: 'text',
          entries: Array.from({ length: 200 }, (_, index) => ({
            op: 'added' as const,
            path: `new:${index + 1}`,
            before: null,
            after: `第 ${index + 1} 行`,
          })),
          added: 320,
          removed: 5,
          changed: 0,
          truncated: true,
        },
      }),
    );
    const [expansions, diff] = sections;
    expect(expansions.lines).toHaveLength(DETAIL_LIMIT);
    expect(expansions.count).toBe(DETAIL_LIMIT + 3);
    expect(expansions.more).toBe(3);
    expect(expansions.note).toBeNull();
    expect(diff.lines).toHaveLength(DETAIL_LIMIT);
    expect(diff.count).toBe(325);
    expect(diff.more).toBe(325 - DETAIL_LIMIT);
    expect(diff.note).toBe('改动太多，接口只给了前一部分。');
  });

  it('不设上限（就地批准的预览开着，T-200）：每段全部列出；接口自己截断了的差异照实写还有几条', () => {
    const many = Array.from({ length: DETAIL_LIMIT + 3 }, (_, index) => ({
      path: `p${index}`,
      before: null,
      after: 'x',
      note: '多了',
    }));
    const entry = withReport({
      problems: [],
      expansions: many,
      narrowings: [],
      diff: {
        format: 'text',
        entries: Array.from({ length: 200 }, (_, index) => ({
          op: 'added' as const,
          path: `new:${index + 1}`,
          before: null,
          after: `第 ${index + 1} 行`,
        })),
        added: 320,
        removed: 5,
        changed: 0,
        truncated: true,
      },
    });
    const [expansions, diff] = entryDetail(entry, Infinity);
    expect(expansions.lines).toHaveLength(DETAIL_LIMIT + 3);
    expect(expansions.count).toBe(DETAIL_LIMIT + 3);
    expect(expansions.more).toBe(0);
    // 接口只给了 200 条：给了的全列，没给的那 125 条照实写明。
    expect(diff.lines).toHaveLength(200);
    expect(diff.count).toBe(325);
    expect(diff.more).toBe(125);
    expect(diff.note).toBe('改动太多，接口只给了前一部分。');
    // 不给上限就是平时的六条。
    expect(entryDetail(entry)[0].lines).toHaveLength(DETAIL_LIMIT);
  });

  it('长的一截截短；空的一截不出；没有校验报告或没有可列的就没有细节', () => {
    const long = '很长的一行规则'.repeat(40);
    const [diff] = entryDetail(
      withReport({
        problems: [],
        expansions: [],
        narrowings: [],
        diff: {
          format: 'text',
          entries: [
            { op: 'removed', path: 'old:3', before: long, after: null },
            { op: 'added', path: 'new:4', before: null, after: '' },
          ],
          added: 1,
          removed: 1,
          changed: 0,
          truncated: false,
        },
      }),
    );
    expect(diff.lines[0].parts).toEqual([brief(long)]);
    expect(Array.from(diff.lines[0].parts[0])).toHaveLength(
      DETAIL_TEXT_MAX + 1,
    );
    expect(diff.lines[1].parts).toEqual([]);

    expect(entryDetail(policyEntry({ ...proposal, report: null }))).toEqual([]);
    expect(
      entryDetail(
        withReport({
          problems: [],
          expansions: [],
          narrowings: [],
          diff: {
            format: 'json',
            entries: [],
            added: 0,
            removed: 0,
            changed: 0,
            truncated: false,
          },
        }),
      ),
    ).toEqual([]);
  });
});

describe('就地细节：黑板任务和工作台待办', () => {
  it('黑板任务列写域：它占着的路径，一条一行', () => {
    const entry = board(TASK.T106);
    if (entry.source !== 'board') throw new Error('应是黑板条目');
    const [paths, ...rest] = entryDetail(entry);
    expect(rest).toEqual([]);
    expect(paths.id).toBe('paths');
    expect(paths.label).toBe('写域');
    expect(paths.count).toBe(entry.item.paths.length);
    expect(paths.lines.map((line) => line.code)).toEqual(
      entry.item.paths.slice(0, DETAIL_LIMIT),
    );
    expect(paths.lines.every((line) => line.mark === null)).toBe(true);
  });

  it('写域多于上限：只列前几条并写明还有几条；一条都没占就没有细节', () => {
    const entry = board(TASK.T106);
    if (entry.source !== 'board') throw new Error('应是黑板条目');
    const wide: PendingEntry = {
      ...entry,
      item: {
        ...entry.item,
        paths: Array.from({ length: DETAIL_LIMIT + 4 }, (_, i) => `目录/${i}`),
      },
    };
    const [paths] = entryDetail(wide);
    expect(paths.lines).toHaveLength(DETAIL_LIMIT);
    expect(paths.more).toBe(4);
    expect(
      entryDetail({ ...entry, item: { ...entry.item, paths: [] } }),
    ).toEqual([]);
  });

  it('工作台待办没有可列的：出处已经写在事实那一行', () => {
    const inbox = entries.find((entry) => entry.source === 'inbox')!;
    expect(entryDetail(inbox)).toEqual([]);
  });
});
