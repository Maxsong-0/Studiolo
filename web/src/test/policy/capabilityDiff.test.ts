import { describe, expect, it } from 'vitest';
import {
  compareCapability,
  serverDirection,
  type CapabilityComparison,
  type CapabilityRow,
} from '../../policy/capabilityDiff';
import { CAPABILITY_FIELDS } from '../../policy/capabilityLabels';

const BASE = {
  ai_type: 'claude',
  read: {
    project_whitelist: ['项目/Studiolo', '项目/旧项目'],
    deny_by_default: true,
    rules_whitelist: ['CLAUDE.md'],
  },
  write: {
    board: false,
    board_projects: [],
    content_patch_only: true,
    content_direct: false,
  },
  policy: { propose_desired: true, activate: false },
  hard_bound_skills: [{ when: '写 React', must_use: 'react-best' }],
};

type Shape = typeof BASE;
const body = (change: (draft: Shape) => void = () => {}) => {
  const draft = structuredClone(BASE);
  change(draft);
  return JSON.stringify(draft);
};

function rows(comparison: CapabilityComparison): readonly CapabilityRow[] {
  if (comparison.kind !== 'rows') throw new Error('读不出结构');
  return comparison.rows;
}
function row(comparison: CapabilityComparison, path: string): CapabilityRow {
  const found = rows(comparison).find((entry) => entry.path === path);
  if (!found) throw new Error(`缺少 ${path} 这一行`);
  return found;
}
const list = (...items: [string, boolean][]) => ({
  kind: 'list' as const,
  items: items.map(([text, onlyHere]) => ({ text, onlyHere })),
});

describe('能力集的「现在／改后」对比（T-198）', () => {
  it('两边一样：每个字段一行、都没改；行序照字段表', () => {
    const same = compareCapability(body(), body());
    expect(same.kind).toBe('rows');
    expect(rows(same).map((entry) => entry.path)).toEqual(
      CAPABILITY_FIELDS.map((field) => field.path),
    );
    expect(rows(same).every((entry) => !entry.changed)).toBe(true);
    expect(same).toMatchObject({ changed: 0 });
  });

  it('名单按集合比：逐项标出只在一边的；先后和重复不算改动', () => {
    const changed = compareCapability(
      body(),
      body((draft) => {
        draft.read.project_whitelist = ['项目/Semiment', '项目/Studiolo'];
      }),
    );
    expect(row(changed, 'read.project_whitelist')).toEqual({
      path: 'read.project_whitelist',
      changed: true,
      before: list(['项目/Studiolo', false], ['项目/旧项目', true]),
      after: list(['项目/Semiment', true], ['项目/Studiolo', false]),
    });
    expect(changed).toMatchObject({ changed: 1 });

    const reordered = compareCapability(
      body(),
      body((draft) => {
        draft.read.project_whitelist = [
          '项目/旧项目',
          '项目/Studiolo',
          '项目/旧项目',
        ];
      }),
    );
    expect(row(reordered, 'read.project_whitelist').changed).toBe(false);
    // 重复的只列一次。
    expect(row(reordered, 'read.project_whitelist').after).toEqual(
      list(['项目/旧项目', false], ['项目/Studiolo', false]),
    );
  });

  it('一边整份都在、只是没写某个名单：按空名单算，和写了空名单一样', () => {
    const omitted = JSON.parse(body()) as Record<string, unknown>;
    delete (omitted.write as Record<string, unknown>).board_projects;
    delete omitted.hard_bound_skills;

    // 没写 ⇄ 空名单：不算改动。
    const empty = compareCapability(JSON.stringify(omitted), body());
    expect(row(empty, 'write.board_projects')).toEqual({
      path: 'write.board_projects',
      changed: false,
      before: list(),
      after: list(),
    });
    // 没写 ⇄ 有内容：那几项都是只在这一边的。
    expect(row(empty, 'hard_bound_skills')).toEqual({
      path: 'hard_bound_skills',
      changed: true,
      before: list(),
      after: list(['写 React → react-best', true]),
    });
  });

  it('单值字段：值不同就是改了；一边没写就照实写「没有」，不补缺省值', () => {
    const flipped = compareCapability(
      body(),
      body((draft) => {
        draft.write.board = true;
      }),
    );
    expect(row(flipped, 'write.board')).toEqual({
      path: 'write.board',
      changed: true,
      before: { kind: 'scalar', value: false },
      after: { kind: 'scalar', value: true },
    });

    const missing = JSON.parse(body()) as Record<string, unknown>;
    delete (missing.policy as Record<string, unknown>).activate;
    expect(
      row(
        compareCapability(body(), JSON.stringify(missing)),
        'policy.activate',
      ),
    ).toEqual({
      path: 'policy.activate',
      changed: true,
      before: { kind: 'scalar', value: false },
      after: { kind: 'absent' },
    });
  });

  it('现在没有生效版本（before 为 null）：「现在」一列都没有，「改后」的每一项都是新给的；空名单不算改动', () => {
    const fresh = compareCapability(
      null,
      body((draft) => {
        draft.hard_bound_skills = [];
      }),
    );
    expect(row(fresh, 'read.project_whitelist')).toEqual({
      path: 'read.project_whitelist',
      changed: true,
      before: { kind: 'absent' },
      after: list(['项目/Studiolo', true], ['项目/旧项目', true]),
    });
    expect(row(fresh, 'write.board')).toMatchObject({
      changed: true,
      before: { kind: 'absent' },
    });
    expect(row(fresh, 'hard_bound_skills')).toEqual({
      path: 'hard_bound_skills',
      changed: false,
      before: { kind: 'absent' },
      after: list(),
    });
    expect(row(fresh, 'write.board_projects').changed).toBe(false);
  });

  it('撤销（after 为 null）：「改后」一列都没有，「现在」的每一项都会收回', () => {
    const retired = compareCapability(body(), null);
    expect(row(retired, 'read.rules_whitelist')).toEqual({
      path: 'read.rules_whitelist',
      changed: true,
      before: list(['CLAUDE.md', true]),
      after: { kind: 'absent' },
    });
    expect(row(retired, 'policy.propose_desired')).toEqual({
      path: 'policy.propose_desired',
      changed: true,
      before: { kind: 'scalar', value: true },
      after: { kind: 'absent' },
    });
    // 两边都没有内容：一行都没有。
    expect(compareCapability(null, null)).toEqual({
      kind: 'rows',
      rows: [],
      changed: 0,
    });
  });

  it('硬绑定写成「触发条件 → skill」（与服务端报告同一写法），键的先后不算改动', () => {
    const swapped = JSON.parse(body()) as Record<string, unknown>;
    swapped.hard_bound_skills = [{ must_use: 'react-best', when: '写 React' }];
    const same = compareCapability(body(), JSON.stringify(swapped));
    expect(row(same, 'hard_bound_skills')).toEqual({
      path: 'hard_bound_skills',
      changed: false,
      before: list(['写 React → react-best', false]),
      after: list(['写 React → react-best', false]),
    });

    // 不是「触发条件／skill」形状的项照实写成 JSON（键按字典序），不硬套。
    swapped.hard_bound_skills = [{ when: '写 React', extra: 1, must_use: 'x' }];
    expect(
      row(compareCapability(null, JSON.stringify(swapped)), 'hard_bound_skills')
        .after,
    ).toEqual(list(['{"extra":1,"must_use":"x","when":"写 React"}', true]));
  });

  it('不认识的字段照样列一行，路径原样，排在认识的字段后面；带点号的键加引号，不和嵌套路径撞', () => {
    const extended = JSON.parse(body()) as Record<string, unknown>;
    extended.quota = { daily: 5 };
    extended['read.project_whitelist'] = 'oops';
    extended.empty = {};
    const comparison = compareCapability(body(), JSON.stringify(extended));
    expect(
      rows(comparison)
        .slice(CAPABILITY_FIELDS.length)
        .map((entry) => entry.path),
    ).toEqual(['quota.daily', '"read.project_whitelist"', 'empty']);
    expect(row(comparison, 'quota.daily')).toEqual({
      path: 'quota.daily',
      changed: true,
      before: { kind: 'absent' },
      after: { kind: 'scalar', value: 5 },
    });
    // 嵌套的那一项没被带点号的同名键盖掉。
    expect(row(comparison, 'read.project_whitelist').changed).toBe(false);
    expect(row(comparison, 'empty').after).toEqual({
      kind: 'scalar',
      value: '{}',
    });
  });

  it('名单对单值（形状变了）：算改了，名单里的项都只在这一边', () => {
    const reshaped = JSON.parse(body()) as Record<string, unknown>;
    (reshaped.read as Record<string, unknown>).rules_whitelist = 'CLAUDE.md';
    expect(
      row(
        compareCapability(body(), JSON.stringify(reshaped)),
        'read.rules_whitelist',
      ),
    ).toEqual({
      path: 'read.rules_whitelist',
      changed: true,
      before: list(['CLAUDE.md', true]),
      after: { kind: 'scalar', value: 'CLAUDE.md' },
    });
  });

  it('任何一边读不出结构就整张表不出，说明是哪一边、为什么，不猜', () => {
    expect(compareCapability('v2 正文', body())).toEqual({
      kind: 'unreadable',
      side: 'before',
      reason: 'not_json',
    });
    expect(compareCapability(body(), '{ "ai_type": ')).toEqual({
      kind: 'unreadable',
      side: 'after',
      reason: 'not_json',
    });
    expect(compareCapability(body(), '  \n')).toEqual({
      kind: 'unreadable',
      side: 'after',
      reason: 'empty',
    });
    for (const text of ['[]', '"claude"', 'null', '3'])
      expect(compareCapability(null, text), text).toEqual({
        kind: 'unreadable',
        side: 'after',
        reason: 'not_object',
      });
    // 两边都读不出：先说基准那一边。
    expect(compareCapability('x', 'y')).toMatchObject({ side: 'before' });
  });
});

describe('扩大、收窄只查服务端的校验报告', () => {
  const report = {
    expansions: [
      {
        path: 'read.project_whitelist',
        before: null,
        after: '项目/Semiment',
        note: '读取范围变宽',
      },
      {
        path: 'write.board',
        before: 'false',
        after: 'true',
        note: '获得黑板写权限',
      },
      {
        path: 'hard_bound_skills',
        before: '写 React → react-best',
        after: null,
        note: '硬绑定被解除：约束放宽',
      },
    ],
    narrowings: [
      {
        path: 'read.project_whitelist',
        before: '项目/旧项目',
        after: null,
        note: '读取范围收窄',
      },
      {
        path: 'policy.propose_desired',
        before: 'true',
        after: 'false',
        note: '能力收回',
      },
    ],
  };

  it('名单里的项：按路径、哪一列、哪一项对上报告里的那一条才标', () => {
    expect(
      serverDirection(
        report,
        'read.project_whitelist',
        'after',
        '项目/Semiment',
      ),
    ).toBe('expansion');
    expect(
      serverDirection(
        report,
        'read.project_whitelist',
        'before',
        '项目/旧项目',
      ),
    ).toBe('narrowing');
    // 去掉硬绑定是放宽约束：服务端判的是扩大，标在「现在」那一列去掉的那一项上。
    expect(
      serverDirection(
        report,
        'hard_bound_skills',
        'before',
        '写 React → react-best',
      ),
    ).toBe('expansion');
    // 新加的项目服务端没算扩大（已被更宽的前缀盖住）：这里也不标。
    expect(
      serverDirection(
        report,
        'read.project_whitelist',
        'after',
        '项目/Studiolo/子目录',
      ),
    ).toBeNull();
    // 同一项放错了列、放错了字段，都不标。
    expect(
      serverDirection(
        report,
        'read.project_whitelist',
        'before',
        '项目/Semiment',
      ),
    ).toBeNull();
    expect(
      serverDirection(report, 'read.rules_whitelist', 'after', '项目/Semiment'),
    ).toBeNull();
  });

  it('单值字段只按路径查', () => {
    expect(serverDirection(report, 'write.board', 'after', null)).toBe(
      'expansion',
    );
    expect(
      serverDirection(report, 'policy.propose_desired', 'after', null),
    ).toBe('narrowing');
    expect(
      serverDirection(report, 'write.content_patch_only', 'after', null),
    ).toBeNull();
  });
});
