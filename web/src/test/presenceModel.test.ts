import { describe, expect, it } from 'vitest';
import { presenceBody } from '../../dev/fixtures/presence';
import { PRESENCE_INVALID, parsePresence } from '../presenceModel';

const NOW = Date.parse('2026-11-24T20:31:00Z');
type Body = ReturnType<typeof presenceBody>;
const body = (change: (draft: Record<string, unknown>) => void = () => {}) => {
  const draft = structuredClone(presenceBody(NOW)) as unknown as Record<
    string,
    unknown
  >;
  change(draft);
  return draft;
};
const row = (change: (row: Record<string, unknown>) => void) =>
  body((draft) => change((draft.sessions as Record<string, unknown>[])[0]));

describe('本机会话的封闭解码', () => {
  it('带全的和只带必带的都照收，没带的不出现', () => {
    const presence = parsePresence(body());
    expect(presence.since).toBe('2026-11-24T14:31:00Z');
    expect(presence.expiresInSecs).toBe(300);
    expect(presence.sessions).toHaveLength(3);
    expect(presence.sessions[0]).toEqual({
      source: 'homelab-status',
      vendor: 'claude_code',
      project: '项目/HomeLab-Center-Docs',
      branch: 'claude/t-092-coding',
      state: 'waiting',
      step: 3,
      total: 8,
      lastAction: 'ask',
      idleSecs: 240,
      ageSecs: 12,
    });
    expect(presence.sessions[2]).toEqual({
      source: 'homelab-status',
      vendor: 'codex',
      project: '项目/Studiolo-Docs',
      state: 'idle',
      ageSecs: 12,
    });
    expect(
      parsePresence(body((draft) => (draft.sessions = []))).sessions,
    ).toEqual([]);
  });

  it.each<[string, (row: Record<string, unknown>) => void]>([
    ['多一个字段', (r) => (r.title = '会话标题')],
    ['带着编号', (r) => (r.key = '0123456789abcdef')],
    ['缺必带的', (r) => delete r.state],
    ['null 不当没带', (r) => (r.branch = null)],
    ['状态不认识', (r) => (r.state = 'unknown')],
    ['哪家不认识', (r) => (r.vendor = 'grok')],
    ['动作不认识', (r) => (r.last_action = 'Bash')],
    ['总数比第几条小', (r) => (r.total = 2)],
    ['只有总数', (r) => delete r.step],
    ['小数', (r) => (r.step = 1.5)],
    ['负数', (r) => (r.idle_secs = -1)],
    ['过期的还在', (r) => (r.age_secs = 301)],
    ['分支带换行', (r) => (r.branch = 'a\nb')],
    ['分支带双向控制符', (r) => (r.branch = 'a‮b')],
    ['分支带零宽符', (r) => (r.branch = 'a​b')],
    ['分支太长', (r) => (r.branch = 'x'.repeat(121))],
    ['项目不是两段', (r) => (r.project = '项目/A/B')],
    ['项目不在「项目」下', (r) => (r.project = '_规则/A')],
    ['来源不是类型名', (r) => (r.source = 'Home Lab')],
  ])('%s：整份判无效', (_name, change) => {
    expect(() => parsePresence(row(change))).toThrow(PRESENCE_INVALID);
  });

  it.each<[string, unknown]>([
    ['不是对象', []],
    ['空', null],
    ['顶层多一个字段', body((d) => (d.host = 'x'))],
    ['顶层缺一个字段', body((d) => delete d.since)],
    ['时刻不是 UTC', body((d) => (d.since = '2026-11-24 20:31'))],
    ['会话不是数组', body((d) => (d.sessions = {}))],
    [
      '条数超上限',
      body(
        (d) =>
          (d.sessions = Array.from(
            { length: 257 },
            () => (presenceBody(NOW) as Body).sessions[2],
          )),
      ),
    ],
  ])('%s：整份判无效', (_name, value) => {
    expect(() => parsePresence(value)).toThrow(PRESENCE_INVALID);
  });
});
