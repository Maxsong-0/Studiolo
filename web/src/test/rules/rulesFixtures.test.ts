import { describe, expect, it } from 'vitest';
import { createFixtureApi } from '../../../dev/fixtureApi';
import { CSRF_TOKEN } from '../../../dev/fixtures/ids';
import {
  desiredEnvelope,
  desiredList,
  policyStatus,
  versionRecord,
} from '../../policyModel';
import { buildRulesModel, withRuleAccess } from '../../rules/rulesModel';

/** 假数据（STUDIOLO_FIXTURES=1）里规则视图要的那几份，全部过真实解码器。 */
function fixtures() {
  const api = createFixtureApi();
  const read = (url: string) =>
    api.handle({ method: 'GET', url, headers: {} }).body;
  const post = (url: string, body: unknown) =>
    api.handle({
      method: 'POST',
      url,
      headers: { 'x-studiolo-request': '1', 'x-csrf-token': CSRF_TOKEN },
      body,
    });
  const status = policyStatus(read('/api/v1/policy'));
  const records = new Map(
    status.entries.map((entry) => [
      entry.version,
      versionRecord(read(`/api/v1/policy/versions/${entry.version}`)),
    ]),
  );
  const desired = desiredList(read('/api/v1/policy/desired'));
  return { read, post, status, records, desired };
}

describe('假数据：规则视图可预览', () => {
  it('首批两条规则都已发布；Claude 的名单列了两条，Codex 缺字段（零规则）', () => {
    const { status, records, desired } = fixtures();
    const model = buildRulesModel({ status, desired, records });
    expect(
      model.rules.map((row) => [row.subject, row.entry?.version, row.readers]),
    ).toEqual([
      ['CLAUDE.md', 1, ['claude']],
      ['通用工作规则.md', 2, ['claude']],
    ]);
    expect(
      model.types.map((column) => [column.type, column.whitelist.kind]),
    ).toEqual([
      ['claude', 'listed'],
      ['codex', 'unset'],
    ]);
    // T-109 的 AI 提案（扩大读取范围）不锁矩阵，只在发布时一并被取代。
    const claude = model.types.find((column) => column.type === 'claude')!;
    expect(claude.ownerDraft).toBeNull();
    expect(claude.others).toBe(1);
    expect(model.unreadable).toEqual([]);
  });

  it('改名单的草案：加一条是扩大项、收一条是收窄项；名单里写了不合法或重复的规则名就校验不过', () => {
    const { post, records, status } = fixtures();
    const codex = status.entries.find((entry) => entry.subject === 'codex')!;
    const codexBody = records.get(codex.version)!.body!;
    const granted = desiredEnvelope(
      post('/api/v1/policy/desired', {
        kind: 'capability_set',
        subject: 'codex',
        body: withRuleAccess(codexBody, 'CLAUDE.md', true),
      }).body,
    );
    expect(granted.status).toBe('validated');
    expect(granted.report?.expansions).toEqual([
      {
        path: 'read.rules_whitelist',
        before: null,
        after: 'CLAUDE.md',
        note: '可读的规则增加',
      },
    ]);
    const claude = status.entries.find((entry) => entry.subject === 'claude')!;
    const narrowed = desiredEnvelope(
      post('/api/v1/policy/desired', {
        kind: 'capability_set',
        subject: 'claude',
        body: withRuleAccess(
          records.get(claude.version)!.body!,
          '通用工作规则.md',
          false,
        ),
      }).body,
    );
    expect(narrowed.report?.expansions).toEqual([]);
    expect(narrowed.report?.narrowings).toEqual([
      {
        path: 'read.rules_whitelist',
        before: '通用工作规则.md',
        after: null,
        note: '可读的规则减少',
      },
    ]);
    const invalid = desiredEnvelope(
      post('/api/v1/policy/desired', {
        kind: 'capability_set',
        subject: 'codex',
        body: withRuleAccess(
          withRuleAccess(codexBody, 'refs/x.md', true),
          'CLAUDE.md',
          true,
        ).replace('"CLAUDE.md"]', '"CLAUDE.md","CLAUDE.md"]'),
      }).body,
    );
    expect(invalid.status).toBe('rejected');
    expect(invalid.report?.problems.map((problem) => problem.path)).toEqual([
      'read.rules_whitelist[0]',
      'read.rules_whitelist[2]',
    ]);
  });
});
