import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { presenceBody } from '../../dev/fixtures/presence';
import { INVISIBLE, PRESENCE_INVALID, parsePresence } from '../presenceModel';

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
    ['分支带谚文填充符', (r) => (r.branch = 'a\u3164b')],
    ['分支带盲文空格', (r) => (r.branch = 'a\u2800b')],
    ['项目带变体选择符', (r) => (r.project = '项目/A\ufe0f')],
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

describe('看不见的字符两端同一张表', () => {
  // 服务端按这张表先拒；网页多拒一个，就会「服务端收了、这一组整个读不到」。
  const rust = readFileSync(
    join(import.meta.dirname, '../../../crates/policy/src/visible.rs'),
    'utf8',
  );
  const ranges = Array.from(
    rust.matchAll(/\(0x([0-9A-F]+), 0x([0-9A-F]+)\)/g),
    (match) => [parseInt(match[1], 16), parseInt(match[2], 16)] as const,
  );
  const listed = (code: number) =>
    ranges.some(([first, last]) => first <= code && code <= last);

  it('网页拒的，服务端都拒', () => {
    expect(ranges).toHaveLength(26);
    const missed: string[] = [];
    let checked = 0;
    for (let code = 0; code <= 0x10ffff; code += 1) {
      if (code >= 0xd800 && code <= 0xdfff) continue;
      const character = String.fromCodePoint(code);
      // 控制字符服务端另有一道（char::is_control）。
      if (!INVISIBLE.test(character) || /\p{Cc}/u.test(character)) continue;
      checked += 1;
      if (!listed(code)) missed.push(code.toString(16));
    }
    expect(missed).toEqual([]);
    expect(checked).toBeGreaterThan(4_000);
  });

  it('表里的网页也都拒', () => {
    const loose: string[] = [];
    for (const [first, last] of ranges)
      for (let code = first; code <= last; code += 1)
        if (!INVISIBLE.test(String.fromCodePoint(code)))
          loose.push(code.toString(16));
    expect(loose).toEqual([]);
  });
});
