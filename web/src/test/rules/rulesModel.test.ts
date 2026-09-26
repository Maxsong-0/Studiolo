import { describe, expect, it } from 'vitest';
import type {
  DesiredRecord,
  EffectiveEntry,
  PolicyKind,
  PolicyStatus,
  VersionRecord,
} from '../../policyModel';
import { draftSummary } from '../../rules/labels';
import {
  accessCell,
  buildRulesModel,
  ruleSubjectProblem,
  ruleWhitelist,
  validRuleSubject,
  withRuleAccess,
} from '../../rules/rulesModel';

const HASH = 'a'.repeat(64);

/** 规则读名单排在 read 中间：改名单时位置不能挪。 */
const CLAUDE = JSON.stringify({
  ai_type: 'claude',
  read: {
    project_whitelist: ['项目/Studiolo', '_规则'],
    rules_whitelist: ['CLAUDE.md'],
    deny_by_default: true,
  },
  write: {
    board: true,
    board_projects: ['项目/Studiolo'],
    content_patch_only: true,
    content_direct: false,
  },
  policy: { propose_desired: true, activate: false },
  hard_bound_skills: [
    { when: '写 React', must_use: 'vercel-react-best-practices' },
  ],
});

/** 该字段出现之前发布的能力集：没有 rules_whitelist。 */
const UNSET = JSON.stringify({
  ai_type: 'codex',
  read: { project_whitelist: ['项目/Studiolo'], deny_by_default: true },
  write: {
    board: false,
    content_patch_only: true,
    content_direct: false,
  },
  policy: { propose_desired: false, activate: false },
});

/** 去掉 read.rules_whitelist 之后的逐字形状（值与键顺序都在字符串里）。 */
function withoutWhitelist(body: string): string {
  const root = JSON.parse(body) as { read: Record<string, unknown> };
  delete root.read.rules_whitelist;
  return JSON.stringify(root);
}

describe('规则名预校验（与后端 validate_rules_subject 同口径）', () => {
  it.each([
    'CLAUDE.md',
    '通用工作规则.md',
    'a.md',
    'Brief 简报站.md',
    `${'规'.repeat(125)}.md`,
  ])('合法：%s', (subject) => {
    expect(ruleSubjectProblem(subject)).toBeNull();
    expect(validRuleSubject(subject)).toBe(true);
  });

  it.each([
    ['', '请填写规则名'],
    ['CLAUDE', '.md 结尾'],
    ['CLAUDE.MD', '.md 结尾'],
    ['.md', '不能以 . 开头'],
    ['.hidden.md', '不能以 . 开头'],
    ['references/x.md', '不能含 /'],
    [`${'规'.repeat(126)}.md`, '最多 128 个字符'],
    ['a\\b.md', '反斜杠'],
    [`a${String.fromCharCode(7)}b.md`, '控制字符'],
    ['a%2Eb.md', '百分号转义'],
    ['a%2fb.md', '百分号转义'],
    [`e${String.fromCodePoint(0x301)}.md`, 'NFC'],
    [`${String.fromCodePoint(0x1100, 0x1161)}.md`, 'NFC'],
  ])('不合法：%j', (subject, reason) => {
    expect(ruleSubjectProblem(subject)).toContain(reason);
    expect(validRuleSubject(subject)).toBe(false);
  });
});

describe('能力集的规则读名单', () => {
  it('缺字段即零规则（未设），列了就按列的，形状不对就读不出、不猜', () => {
    expect(ruleWhitelist(UNSET)).toEqual({ kind: 'unset' });
    expect(ruleWhitelist(CLAUDE)).toEqual({
      kind: 'listed',
      rules: ['CLAUDE.md'],
    });
    for (const body of [
      null,
      '不是 JSON',
      '[]',
      '{"ai_type":"x"}',
      '{"read":{"rules_whitelist":"CLAUDE.md"}}',
      '{"read":{"rules_whitelist":[1]}}',
    ])
      expect(ruleWhitelist(body)).toEqual({ kind: 'invalid' });
  });

  it('切格子只改 read.rules_whitelist：其余字段、值与键顺序逐字不变，名单原位替换', () => {
    const granted = withRuleAccess(CLAUDE, '通用工作规则.md', true);
    expect(granted).toBe(
      CLAUDE.replace(
        '"rules_whitelist":["CLAUDE.md"]',
        '"rules_whitelist":["CLAUDE.md","通用工作规则.md"]',
      ),
    );
    expect(withoutWhitelist(granted)).toBe(withoutWhitelist(CLAUDE));
    const revoked = withRuleAccess(CLAUDE, 'CLAUDE.md', false);
    expect(revoked).toBe(
      CLAUDE.replace('"rules_whitelist":["CLAUDE.md"]', '"rules_whitelist":[]'),
    );
    expect(withoutWhitelist(revoked)).toBe(withoutWhitelist(CLAUDE));
    // 已经能读再授予、本来不能读再收回：正文原样。
    expect(withRuleAccess(CLAUDE, 'CLAUDE.md', true)).toBe(CLAUDE);
  });

  it('缺字段时在 read 末尾补上名单，别的一字不动', () => {
    const granted = withRuleAccess(UNSET, 'CLAUDE.md', true);
    expect(granted).toBe(
      UNSET.replace(
        '"deny_by_default":true}',
        '"deny_by_default":true,"rules_whitelist":["CLAUDE.md"]}',
      ),
    );
    expect(withoutWhitelist(granted)).toBe(UNSET);
  });

  it('多行排版的正文照样只改名单：值与键顺序不变', () => {
    const pretty = JSON.stringify(JSON.parse(CLAUDE), null, 2);
    const granted = withRuleAccess(pretty, '通用工作规则.md', true);
    expect(withoutWhitelist(granted)).toBe(withoutWhitelist(CLAUDE));
    expect(ruleWhitelist(granted)).toEqual({
      kind: 'listed',
      rules: ['CLAUDE.md', '通用工作规则.md'],
    });
  });

  it('正文读不出或名单形状不对就拒绝改，不在猜出来的结构上写', () => {
    expect(() => withRuleAccess('不是 JSON', 'CLAUDE.md', true)).toThrow(
      '不是合法的 JSON',
    );
    expect(() => withRuleAccess('{"ai_type":"x"}', 'CLAUDE.md', true)).toThrow(
      '缺少 read',
    );
    expect(() =>
      withRuleAccess(
        '{"read":{"rules_whitelist":"CLAUDE.md"}}',
        'CLAUDE.md',
        true,
      ),
    ).toThrow('不是规则名列表');
  });
});

function entry(
  kind: PolicyKind,
  subject: string,
  version: number,
): EffectiveEntry {
  return {
    kind,
    subject,
    version,
    body_sha256: HASH,
    activated_at: '2026-09-20T08:00:00Z',
  };
}

function versionOf(item: EffectiveEntry, body: string | null): VersionRecord {
  return {
    meta: {
      version: item.version,
      kind: item.kind,
      subject: item.subject,
      retired: false,
      body_sha256: item.body_sha256,
      effective_hash: HASH,
      previous_effective_hash: HASH,
      activated_at: item.activated_at,
      activated_by: 'owner',
      desired_id: null,
      rollback_of: null,
      expansions: [],
    },
    body,
  };
}

function draft(overrides: Partial<DesiredRecord>): DesiredRecord {
  return {
    id: '01ABCDEFGHJKMNPQRSTVWXYZ00',
    kind: 'rules',
    subject: 'CLAUDE.md',
    body: '# 草案\n',
    body_sha256: HASH,
    submitter_kind: 'owner',
    agent_type: null,
    agent_instance: null,
    submitted_at: '2026-09-21T08:00:00Z',
    status: 'validated',
    report: null,
    validated_at: null,
    activated_version: null,
    ...overrides,
  };
}

function statusOf(
  entries: EffectiveEntry[],
  revoked: string[] = [],
): PolicyStatus {
  return {
    effective_hash: HASH,
    entries,
    global_revoked: false,
    pending_desired: 0,
    revocations: revoked.map((target, index) => ({
      seq: index + 1,
      level: 'type' as const,
      target,
      revoked_at: '2026-09-20T00:00:00Z',
      revoked_by: 'owner',
      reason: '演练',
      released_at: null,
      released_by: null,
    })),
    l3_available: true,
  };
}

describe('规则行与读权限列', () => {
  const rule = entry('rules', 'CLAUDE.md', 1);
  const general = entry('rules', '通用工作规则.md', 2);
  const claude = entry('capability_set', 'claude', 3);
  const codex = entry('capability_set', 'codex', 4);
  const broken = entry('capability_set', 'broken', 5);
  const claudeBody = withRuleAccess(
    withRuleAccess(CLAUDE, '通用工作规则.md', true),
    'ghost.md',
    true,
  );
  const records = new Map(
    [
      versionOf(rule, '# CLAUDE\n'),
      versionOf(general, '# 通用\n'),
      versionOf(claude, claudeBody),
      versionOf(codex, UNSET),
      versionOf(broken, '{"read":{"rules_whitelist":{}}}'),
    ].map((item) => [item.meta.version, item]),
  );
  const ownerDraft = draft({
    id: '01ABCDEFGHJKMNPQRSTVWXYZ02',
    kind: 'capability_set',
    subject: 'codex',
    body: withRuleAccess(UNSET, 'CLAUDE.md', true),
    submitted_at: '2026-09-22T08:00:00Z',
  });
  const agentProposal = draft({
    id: '01ABCDEFGHJKMNPQRSTVWXYZ03',
    kind: 'capability_set',
    subject: 'codex',
    body: UNSET,
    submitter_kind: 'agent',
    agent_type: 'codex',
    agent_instance: '01AGENT',
    status: 'pending',
  });
  const model = buildRulesModel({
    status: statusOf([rule, general, claude, codex, broken], ['codex']),
    desired: [
      agentProposal,
      ownerDraft,
      draft({ subject: '新规则.md' }),
      draft({ id: '01ABCDEFGHJKMNPQRSTVWXYZ09', status: 'withdrawn' }),
    ],
    records,
  });

  it('行：生效规则、只有草案的新规则与只在读名单里的规则都列出，按码点序；读者只算名单里列了它的类型', () => {
    expect(
      model.rules.map((row) => [
        row.subject,
        row.entry?.version ?? null,
        row.readers,
        row.drafts.map((item) => item.subject),
      ]),
    ).toEqual([
      ['CLAUDE.md', 1, ['claude'], []],
      ['ghost.md', null, ['claude'], []],
      ['新规则.md', null, [], ['新规则.md']],
      ['通用工作规则.md', 2, ['claude'], []],
    ]);
    // 撤回的草案不算未发布草案。
    expect(model.rules[0].drafts).toEqual([]);
  });

  it('列：缺字段即零规则；名单读不出的类型单列出来，不算进任何读者', () => {
    const byType = new Map(model.types.map((column) => [column.type, column]));
    expect([...byType.keys()]).toEqual(['broken', 'claude', 'codex']);
    expect(byType.get('codex')?.whitelist).toEqual({ kind: 'unset' });
    expect(byType.get('codex')?.revoked).toBe(true);
    expect(byType.get('broken')?.whitelist).toEqual({ kind: 'invalid' });
    expect(model.unreadable).toEqual(['broken']);
  });

  it('你提交的能力集草案让这一列按草案显示并锁住；AI 提案只计数，发布时一并被取代', () => {
    const codexColumn = model.types.find((column) => column.type === 'codex')!;
    expect(codexColumn.ownerDraft?.id).toBe(ownerDraft.id);
    expect(codexColumn.others).toBe(1);
    expect(accessCell(codexColumn, 'CLAUDE.md')).toEqual({
      effective: false,
      shown: true,
      pending: true,
      locked: true,
    });
    expect(accessCell(codexColumn, '通用工作规则.md')).toEqual({
      effective: false,
      shown: false,
      pending: false,
      locked: true,
    });
    const claudeColumn = model.types.find(
      (column) => column.type === 'claude',
    )!;
    expect(accessCell(claudeColumn, 'CLAUDE.md')).toEqual({
      effective: true,
      shown: true,
      pending: false,
      locked: false,
    });
    const brokenColumn = model.types.find(
      (column) => column.type === 'broken',
    )!;
    expect(accessCell(brokenColumn, 'CLAUDE.md')).toMatchObject({
      effective: null,
      shown: null,
      locked: true,
    });
  });

  it('列表的「未发布草案」：校验未通过与 AI 提案是红色（等你），自己的草案是待定', () => {
    expect(draftSummary([])).toBeNull();
    expect(draftSummary([draft({})])).toEqual({
      state: 'pending',
      text: '未发布 1 份',
    });
    expect(draftSummary([draft({}), agentProposal])).toEqual({
      state: 'alert',
      text: 'AI 提案待复核',
    });
    expect(draftSummary([draft({ status: 'rejected' })])).toEqual({
      state: 'alert',
      text: '草案校验未通过',
    });
  });
});
