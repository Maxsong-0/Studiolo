import { PassThrough } from 'node:stream';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createFixtureApi } from '../../dev/fixtureApi';
import {
  FIXTURE_ENV,
  fixturesEnabled,
  studioloFixtures,
} from '../../dev/fixturePlugin';
import { validateDocument, validateWorkbench } from '../validate';
import { parseBoardOverview } from '../boardOverviewModel';
import { validFreshness } from '../freshness';
import { authentication, sessionList, sessionPair } from '../sessionModel';
import {
  factorState,
  recentAuthentication,
  recoveryCodes,
} from '../factorModel';
import { boardItem, boardItems, boardLease } from '../boardModel';
import type { BoardItem } from '../boardModel';
import { timelinePage } from '../projectTimelineModel';
import {
  candidatePatch,
  executionStatus,
  landingCommand,
  landingResult,
  replayPage,
  reviewTemplate,
} from '../executionModel';
import {
  activateOutcome,
  desiredEnvelope,
  desiredList,
  policyStatus,
  revokeOutcome,
  versionList,
  versionRecord,
} from '../policyModel';
import { parseHealth } from '../healthApi';
import { validateTree } from '../treeQuery';
import { validateSearch } from '../searchQuery';
import { validateProjectOperations } from '../projectOperations';
import { canonicalDocument, saveReceipt } from '../canonicalModel';
import { historyFile, historyPage } from '../historyModel';
import {
  fileOperationCapability,
  fileOperationReceipt,
  tombstonePage,
} from '../fileOperationValidation';
import type { TombstoneEntry } from '../fileOperationModel';
import type { TreeEntry } from '../treeTypes';
import type { Workbench } from '../types';

/** 固定在「今天」20:31（本地时间）之后：时间带正好是 08:00–20:00。 */
const NOW = (() => {
  const day = new Date(2026, 8, 22, 20, 31, 0, 0);
  return day.getTime();
})();
const HOUR = 3_600_000;
const CSRF = /^c2\.[0-9a-f]{16}\.[0-9a-f]{64}$/;

function setup(now = () => NOW) {
  const api = createFixtureApi({ now });
  const get = (url: string) =>
    api.handle({
      method: 'GET',
      url,
      headers: { 'x-studiolo-request': '1' },
    });
  let csrf = '';
  const post = (url: string, body: unknown) =>
    api.handle({
      method: 'POST',
      url,
      headers: { 'x-studiolo-request': '1', 'x-csrf-token': csrf },
      body,
    });
  const session = get('/api/v1/session');
  csrf = sessionPair(session.body).csrf;
  expect(csrf).toMatch(CSRF);
  const read = (url: string) => {
    const response = get(url);
    expect(response.status, url).toBe(200);
    return response.body;
  };
  return { api, get, post, read };
}

const query = (params: Record<string, string>) =>
  new URLSearchParams(params).toString();

function workbench(read: (url: string) => unknown): Workbench {
  return validateWorkbench(read('/api/workbench'));
}

describe('假数据：会话与首屏', () => {
  it('会话模式：bootstrap、会话对、会话列表、安全状态都过真实解码器', () => {
    const { get, read } = setup();
    expect(authentication(get('/api/v1/bootstrap').body)).toBe('session');
    const pair = sessionPair(read('/api/v1/session'));
    expect(pair.ownerScope).toMatch(/^[0-9a-f]{64}$/);
    expect(pair.session.absolute_expires_at_ms).toBeGreaterThan(NOW);
    expect(sessionList(read('/api/v1/sessions'))).toHaveLength(2);
    const factors = factorState(read('/api/v1/auth/state'));
    expect(factors?.recent_authentication).toBeNull();
  });

  it('工作台：八个项目、五条待办、「同步偏慢」降级，刷新后追平', () => {
    const { read } = setup();
    const data = workbench(read);
    expect(data.projects.map((project) => project.name)).toEqual([
      'Studiolo',
      'Semiment',
      'Brief 简报站',
      'AI 网关门户',
      'DocReader',
      'NAS 网关',
      'Tillerwright',
      'AI Quota Flow',
    ]);
    expect(data.inbox).toHaveLength(5);
    expect(data.counts.inbox.value).toBe(5);
    expect(data.counts.projects.value).toBe(8);
    expect(data.freshness).toMatchObject({ stale: true, reason: 'changed' });
    // 生命周期只出现后端闭集里的值；闭集外的（Tillerwright「可用」）是 null 并带诊断。
    for (const project of data.projects)
      expect([
        null,
        '活跃',
        '暂停',
        '已交付维护',
        '静态资料',
        '候选',
        '未建档',
      ]).toContain(project.lifecycle.value);
    const tillerwright = data.projects.find((p) => p.name === 'Tillerwright')!;
    expect(tillerwright.lifecycle.value).toBeNull();
    expect(tillerwright.diagnostics.join('\n')).toContain('生命周期不在闭集内');
    // 未关闭 BUG：没有 BUG 档的项目不可得，汇总显式不可得而不是 0。
    expect(data.counts.open_bugs.value).toBeNull();
    expect(data.counts.open_bugs.unavailable.length).toBeGreaterThan(0);
    const refreshed = validateWorkbench(read('/api/workbench?refresh=true'));
    expect(refreshed.freshness?.stale).toBe(false);
    expect(validFreshness(refreshed.freshness)).toBe(true);
  });

  it('待办与项目字段的来源行号指向原文里真实存在的那一行', () => {
    const { read } = setup();
    const data = workbench(read);
    const line = (path: string, number: number) => {
      const document = validateDocument(
        read(
          `/api/document?${query({ path, generation: String(data.generation) })}`,
        ),
      );
      return document.content.split('\n')[number - 1];
    };
    for (const action of data.inbox) {
      expect(line(action.source.path, action.source.line)).toContain(
        action.title,
      );
      expect(action.id).toBe(
        `${action.source.path}:${action.source.line}:${action.kind}`,
      );
    }
    for (const project of data.projects)
      if (project.lifecycle.source)
        expect(
          line(project.lifecycle.source.path, project.lifecycle.source.line),
        ).toContain(String(project.lifecycle.value));
  });
});

describe('假数据：协作黑板、汇总、回放与执行', () => {
  const studiolo = '项目/Studiolo';

  it('每个项目的黑板都过封闭解码器；Studiolo 七条任务，T-092 写域落在 T-130 的 web 里', () => {
    const { read } = setup();
    const data = workbench(read);
    const all: BoardItem[] = [];
    for (const project of data.projects)
      all.push(
        ...boardItems(
          read(`/api/v1/board?${query({ project: project.root })}`),
          project.root,
        ),
      );
    const board = all.filter((item) => item.project === studiolo);
    expect(board.map((item) => [item.title.slice(0, 5), item.status])).toEqual([
      ['T-046', 'claimed'],
      ['T-092', 'approved'],
      ['T-093', 'needs_revision'],
      ['T-106', 'needs_review'],
      ['T-108', 'needs_review'],
      ['T-109', 'running'],
      ['T-130', 'running'],
    ]);
    const t130 = board.find((item) => item.title.startsWith('T-130'))!;
    const t092 = board.find((item) => item.title.startsWith('T-092'))!;
    expect(t130.paths).toEqual(['web']);
    expect(t092.paths[0].startsWith('web/')).toBe(true);
    for (const item of board.filter((row) => row.attempt?.active))
      expect(boardLease(item, NOW)).toBe('current');
    const t046 = board.find((item) => item.title.startsWith('T-046'))!;
    expect(t046.oldPolicy).toBe(true);
    expect(NOW - t046.progress!.lastProgressMs).toBeGreaterThan(3 * 24 * HOUR);
    expect(all.find((item) => item.project !== studiolo)?.receipt?.status).toBe(
      'completed',
    );
  });

  it('开发服务器开着几个小时后：在跑任务租约仍有效，进度时间仍对齐启动时刻', () => {
    let now = NOW;
    const { read } = setup(() => now);
    const url = `/api/v1/board?${query({ project: studiolo })}`;
    const before = boardItems(read(url), studiolo);
    now = NOW + 6 * HOUR;
    const after = boardItems(read(url), studiolo);
    for (const item of after.filter((row) => row.attempt?.active))
      expect(boardLease(item, now)).toBe('current');
    expect(after.map((item) => item.progress?.lastProgressMs)).toEqual(
      before.map((item) => item.progress?.lastProgressMs),
    );
  });

  it('跨项目汇总：字段集合与设计规范第 10 节逐一相等', () => {
    const { read } = setup();
    const value = read('/api/v1/board/overview') as Record<string, unknown>;
    // A3 的封闭解码器是最终裁判：假数据必须原样通过它。
    expect(() => parseBoardOverview(value, 24)).not.toThrow();
    const keys = (row: unknown) => Object.keys(row as object).sort();
    expect(keys(value)).toEqual(
      [
        'generated_at',
        'window_from',
        'window_to',
        'policy_hash',
        'items',
        'items_truncated',
        'events',
        'events_truncated',
      ].sort(),
    );
    expect(Date.parse(String(value.window_to))).toBe(NOW);
    expect(
      Date.parse(String(value.window_to)) -
        Date.parse(String(value.window_from)),
    ).toBe(24 * HOUR);
    expect(value.policy_hash).toMatch(/^[0-9a-f]{64}$/);
    const items = value.items as Record<string, unknown>[];
    expect(items.length).toBeGreaterThanOrEqual(8);
    for (const item of items) {
      expect(keys(item)).toEqual(
        [
          'id',
          'title',
          'project',
          'status',
          'parent',
          'adapter',
          'holder',
          'holder_lineage',
          'paths',
          'attempt',
          'progress',
          'receipt_status',
        ].sort(),
      );
      expect(item.id).toMatch(/^[0-7][0-9A-HJKMNP-TV-Z]{25}$/);
      for (const node of item.holder_lineage as unknown[])
        expect(keys(node)).toEqual(['agent_type', 'id']);
      if (item.attempt !== null)
        expect(keys(item.attempt)).toEqual(
          ['active', 'lease_until_ms', 'phase', 'worker'].sort(),
        );
      if (item.progress !== null)
        expect(keys(item.progress)).toEqual(
          ['edits', 'last_progress_ms', 'tools'].sort(),
        );
      expect([null, 'completed', 'failed', 'stopped', 'abandoned']).toContain(
        item.receipt_status,
      );
    }
    const events = value.events as Record<string, unknown>[];
    expect(events.length).toBeGreaterThan(20);
    for (const event of events)
      expect(keys(event)).toEqual(
        ['action', 'at', 'attempt_id', 'phase', 'project', 'task_id'].sort(),
      );
    const times = events.map((event) => Date.parse(String(event.at)));
    expect(times).toEqual([...times].sort((a, b) => a - b));
    // 时间带是当天 08:00–20:00，NOW 是 20:31。
    expect(Math.min(...times)).toBeGreaterThanOrEqual(NOW - 12.6 * HOUR);
    expect(Math.max(...times)).toBeLessThanOrEqual(NOW);
    // 窗口收窄只影响事件与「已结束但窗口内有事件」的条目。
    const narrow = read('/api/v1/board/overview?window_hours=1') as {
      events: unknown[];
      items: unknown[];
    };
    expect(narrow.events.length).toBeLessThan(events.length);
    expect(narrow.items.length).toBe(7);
  });

  it('跨项目汇总拒绝越界窗口与未知参数', () => {
    const { get } = setup();
    expect(get('/api/v1/board/overview?window_hours=0').status).toBe(400);
    expect(get('/api/v1/board/overview?window_hours=169').status).toBe(400);
    expect(get('/api/v1/board/overview?project=x').status).toBe(400);
  });

  it('项目时间回放：事件、当日日志与无日期日志计数都过解码器', () => {
    const { read } = setup();
    const to = new Date(NOW).toISOString();
    const from = new Date(NOW - 24 * HOUR).toISOString();
    const page = timelinePage(
      read(`/api/v1/board/timeline?${query({ project: studiolo, from, to })}`),
      studiolo,
      from,
      to,
    );
    expect(page.events.length).toBeGreaterThan(30);
    expect(page.logs.length).toBe(2);
    expect(page.undated_logs).toBe(1);
    const after = page.events[10].offset;
    const next = timelinePage(
      read(
        `/api/v1/board/timeline?${query({ project: studiolo, from, to, after })}`,
      ),
      studiolo,
      from,
      to,
    );
    expect(
      next.events.every((event) => BigInt(event.offset) > BigInt(after)),
    ).toBe(true);
  });

  it('执行：五个执行体；T-108 回放走到九步里的第七步，两个候选产物', () => {
    const { read } = setup();
    const status = executionStatus(read('/api/v1/execution'));
    expect(status).toMatchObject({ configured: true, frozen: false });
    expect(status.runs).toHaveLength(5);
    const board = boardItems(
      read(`/api/v1/board?${query({ project: studiolo })}`),
      studiolo,
    );
    for (const run of status.runs)
      expect(board.find((item) => item.id === run.task_id)?.attempt?.id).toBe(
        run.attempt_id,
      );
    for (const task of board) {
      const replay = replayPage(
        read(`/api/v1/board/replay?${query({ task_id: task.id })}`),
        task.id,
        studiolo,
        null,
      );
      expect(replay.task.id).toBe(task.id);
    }
    const t108 = board.find((item) => item.title.startsWith('T-108'))!;
    const replay = replayPage(
      read(`/api/v1/board/replay?${query({ task_id: t108.id })}`),
      t108.id,
      studiolo,
      null,
    );
    expect(new Set(replay.events.map((event) => event.action)).size).toBe(7);
    expect(t108.candidateReceipt?.artifacts).toHaveLength(2);
    expect(candidatePatch(t108)).not.toBeNull();
  });
});

describe('假数据：策略、健康、文档与项目运作', () => {
  it('策略平面：现行四版、待处理 desired 一条（三项扩大）、实例与类型两级吊销生效', () => {
    const { read } = setup();
    const status = policyStatus(read('/api/v1/policy'));
    expect(status.entries).toHaveLength(4);
    expect(status.pending_desired).toBe(1);
    expect(status.global_revoked).toBe(false);
    expect(status.revocations.map((row) => row.level)).toEqual([
      'instance',
      'type',
    ]);
    const desired = desiredList(read('/api/v1/policy/desired'));
    expect(desired).toHaveLength(1);
    expect(desired[0]).toMatchObject({
      status: 'pending',
      submitter_kind: 'agent',
    });
    expect(desired[0].report?.expansions).toHaveLength(3);
    const versions = versionList(read('/api/v1/policy/versions'));
    expect(versions.map((meta) => meta.version)).toEqual([4, 3, 2, 1]);
    for (const meta of versions)
      expect(
        versionRecord(read(`/api/v1/policy/versions/${meta.version}`)).meta
          .version,
      ).toBe(meta.version);
  });

  it('健康：整体降级，副本正常、同步「每轮 85 秒，时限 120 秒」降级、会话正常', () => {
    const { read } = setup();
    const report = parseHealth(read('/api/v1/health'));
    expect(report.overall).toBe('degraded');
    expect(report.evidenceComplete).toBe(true);
    const state = (code: string) =>
      report.sections.find((section) => section.code === code);
    expect(state('replica-freshness')?.state).toBe('normal');
    expect(state('sync-round')).toMatchObject({ state: 'degraded' });
    expect(state('sync-round')?.detail).toContain('每轮 85 秒，时限 120 秒');
    expect(state('session')?.state).toBe('normal');
  });

  it('目录树逐页、每篇原文与导航、搜索都过解码器', () => {
    const { read } = setup();
    const { generation } = workbench(read);
    const markdown: string[] = [];
    const walk = (path: string) => {
      const page = validateTree(
        read(
          `/api/tree?${query({ path, offset: '0', generation: String(generation) })}`,
        ),
      );
      for (const entry of page.entries as TreeEntry[])
        if (entry.kind === 'directory') walk(entry.path);
        else if (entry.kind === 'markdown') markdown.push(entry.path);
    };
    walk('');
    expect(markdown.length).toBeGreaterThan(25);
    const statuses = new Set<string>();
    for (const path of markdown) {
      const document = validateDocument(
        read(
          `/api/document?${query({ path, generation: String(generation) })}`,
        ),
      );
      document.navigation.outgoing.forEach((link) => statuses.add(link.status));
    }
    expect([...statuses].sort()).toEqual(['ambiguous', 'missing', 'resolved']);
    for (const mode of ['full_text', 'path'])
      expect(
        validateSearch(
          read(
            `/api/search?${query({ q: 'Studiolo', mode, generation: String(generation) })}`,
          ),
        ).hits.length,
      ).toBeGreaterThan(0);
  });

  it('搜索与服务端同一口径：空查询合法（路径列举全部、全文为空），只拒超长与控制字符', () => {
    const { get, read } = setup();
    const { generation } = workbench(read);
    const search = (q: string, mode: string) =>
      `/api/search?${query({ q, mode, generation: String(generation) })}`;
    expect(
      validateSearch(read(search('', 'path'))).hits.length,
    ).toBeGreaterThan(0);
    expect(validateSearch(read(search('   ', 'full_text'))).hits).toEqual([]);
    expect(get(search('x'.repeat(257), 'path')).status).toBe(400);
    expect(get(search('a\u0007b', 'full_text')).status).toBe(400);
  });

  it('项目运作：Studiolo 声明、漂移、架构、Cargo 与 Rust 调用齐全，其余项目按「不可用」给出', () => {
    const { read } = setup();
    const data = workbench(read);
    for (const project of data.projects) {
      const operations = validateProjectOperations(
        read(
          `/api/project-operations?${query({ project: project.root, generation: String(data.generation) })}`,
        ),
      );
      if (project.name === 'Studiolo') {
        expect(operations.declaration_status).toBe('ready');
        expect(operations.architecture?.status).toBe('ready');
        expect(operations.cargo_dependencies?.graph?.packages).toHaveLength(23);
        expect(operations.rust_call_sites?.links?.associations).toHaveLength(
          16,
        );
        expect(
          operations.drift.filter((item) => item.status === 'undocumented'),
        ).toHaveLength(19);
      } else expect(operations.declaration_status).toBe('unavailable');
    }
  });
});

describe('假数据：写操作走得通', () => {
  it('发布策略：先 403 要二次认证，认证后发布为 v5，旧任务改按旧版规则执行', () => {
    const { get, post, read } = setup();
    const desired = desiredList(read('/api/v1/policy/desired'))[0];
    const status = policyStatus(read('/api/v1/policy'));
    const request = {
      desired_id: desired.id,
      expected_effective_hash: status.effective_hash,
    };
    const denied = post('/api/v1/policy/activate', request);
    expect(denied.status).toBe(403);
    expect(denied.body).toMatchObject({ code: 'reauthentication_required' });
    expect(post('/api/v1/auth/step-up', { code: '000000' }).status).toBe(403);
    const recent = recentAuthentication(
      post('/api/v1/auth/step-up', { code: '482913' }).body,
    );
    expect(recent.expires_at_ms - recent.verified_at_ms).toBe(300_000);
    expect(
      factorState(read('/api/v1/auth/state'))?.recent_authentication,
    ).toEqual(recent);
    const outcome = activateOutcome(
      post('/api/v1/policy/activate', request).body,
    );
    expect(outcome.kind).toBe('activated');
    if (outcome.kind === 'activated')
      expect(outcome.activation.version).toBe(5);
    const after = policyStatus(read('/api/v1/policy'));
    expect(after.effective_hash).not.toBe(status.effective_hash);
    expect(after.pending_desired).toBe(0);
    const board = boardItems(
      read(`/api/v1/board?${query({ project: '项目/Studiolo' })}`),
      '项目/Studiolo',
    );
    expect(board.every((item) => item.oldPolicy)).toBe(true);
    expect(
      recoveryCodes(post('/api/v1/auth/recovery-codes', {}).body),
    ).toHaveLength(10);
    expect(get('/api/v1/policy/versions/9').status).toBe(404);
  });

  it('同一对象几份未发布草案：列表标出先后和挡住发布的；同一提交者更新的才挡，发布只取代比它早的（T-181）', () => {
    const { post, read } = setup();
    const proposal = desiredList(read('/api/v1/policy/desired'))[0];
    expect(proposal.submitter_kind).toBe('agent');
    const current = versionRecord(read('/api/v1/policy/versions/3'));
    // 主人在 AI 提案之后先后交两份能力集草案，只改规则读名单。
    const submit = (rules: string[]) => {
      const body = JSON.parse(current.body ?? '{}') as {
        read: { rules_whitelist: string[] };
      };
      body.read.rules_whitelist = rules;
      return desiredEnvelope(
        post('/api/v1/policy/desired', {
          kind: 'capability_set',
          subject: 'claude',
          body: JSON.stringify(body),
        }).body,
      );
    };
    const owner = submit(['CLAUDE.md']);
    const latest = submit(['通用工作规则.md']);
    expect([owner.status, latest.status]).toEqual(['validated', 'validated']);
    expect(
      desiredList(read('/api/v1/policy/desired')).map((entry) => [
        entry.id,
        entry.newer_open,
        entry.blocked_by,
      ]),
    ).toEqual([
      [latest.id, [], []],
      [owner.id, [latest.id], [latest.id]],
      [proposal.id, [owner.id, latest.id], []],
    ]);
    const before = policyStatus(read('/api/v1/policy')).effective_hash;
    recentAuthentication(post('/api/v1/auth/step-up', { code: '482913' }).body);
    // 主人自己还有更新的：较早那份回 409、什么都不改。
    expect(
      post('/api/v1/policy/activate', {
        desired_id: owner.id,
        expected_effective_hash: before,
      }),
    ).toMatchObject({
      status: 409,
      body: {
        code: 'newer_desired_same_submitter',
        newer_desired_ids: [latest.id],
      },
    });
    expect(policyStatus(read('/api/v1/policy')).effective_hash).toBe(before);
    expect(
      desiredList(read('/api/v1/policy/desired')).map((entry) => entry.status),
    ).toEqual(['validated', 'validated', 'pending']);
    // AI 的提案较早，但更新的两份都是主人交的，不挡它；发布后主人那两份留着待处理。
    expect(
      activateOutcome(
        post('/api/v1/policy/activate', {
          desired_id: proposal.id,
          expected_effective_hash: before,
        }).body,
      ).kind,
    ).toBe('activated');
    expect(
      desiredList(read('/api/v1/policy/desired')).map((entry) => [
        entry.id,
        entry.status,
      ]),
    ).toEqual([
      [latest.id, 'validated'],
      [owner.id, 'validated'],
      [proposal.id, 'activated'],
    ]);
    // 主人最新那份照发，比它早的那份标为已取代。
    const after = policyStatus(read('/api/v1/policy')).effective_hash;
    expect(
      activateOutcome(
        post('/api/v1/policy/activate', {
          desired_id: latest.id,
          expected_effective_hash: after,
        }).body,
      ).kind,
    ).toBe('activated');
    expect(
      desiredList(read('/api/v1/policy/desired')).map((entry) => [
        entry.id,
        entry.status,
      ]),
    ).toEqual([
      [latest.id, 'activated'],
      [owner.id, 'superseded'],
      [proposal.id, 'activated'],
    ]);
  });

  it('提交 desired、重新校验、撤回、吊销都返回解码器接受的形状', () => {
    const { post } = setup();
    const created = desiredEnvelope(
      post('/api/v1/policy/desired', {
        kind: 'skills',
        subject: 'replica-sync',
        body: JSON.stringify({
          name: 'replica-sync',
          summary: '副本同步排障步骤',
          triggers: ['副本同步超时'],
          markdown: '# 副本同步\n\n先看每轮耗时，再看时限。',
        }),
      }).body,
    );
    expect(created.status).toBe('validated');
    const invalid = desiredEnvelope(
      post('/api/v1/policy/desired', {
        kind: 'capability_set',
        subject: 'claude',
        body: '{"ai_type":"claude"}',
      }).body,
    );
    expect(invalid).toMatchObject({ status: 'rejected' });
    expect(invalid.report?.ok).toBe(false);
    expect(
      desiredEnvelope(
        post('/api/v1/policy/validate', { desired_id: created.id }).body,
      ).status,
    ).toBe('validated');
    expect(
      desiredEnvelope(
        post('/api/v1/policy/withdraw', { desired_id: created.id }).body,
      ).status,
    ).toBe('withdrawn');
    expect(
      revokeOutcome(
        post('/api/v1/agents/revoke', {
          level: 'type',
          target: 'consumer',
          reason: '重复吊销',
        }).body,
      ),
    ).toEqual({ created: false, level: 'type', target: 'consumer' });
    expect(
      revokeOutcome(
        post('/api/v1/agents/revoke', { level: 'global', reason: '演练' }).body,
      ),
    ).toEqual({ created: true, level: 'global', target: '*' });
  });

  it('执行：落地与审阅要二次认证；落地后以 completed 收尾，任务变为 succeeded', () => {
    const { post, read } = setup();
    const project = '项目/Studiolo';
    const board = boardItems(
      read(`/api/v1/board?${query({ project })}`),
      project,
    );
    const task = board.find((item) => item.title.startsWith('T-108'))!;
    const command = landingCommand(
      task,
      'a'.repeat(40),
      candidatePatch(task)!.sha256,
    );
    const denied = post('/api/v1/execution/command', command);
    expect(denied.status).toBe(409);
    expect(String((denied.body as { error: string }).error)).toMatch(
      /二次认证/,
    );
    post('/api/v1/auth/step-up', { code: '120934' });
    const landed = landingResult(
      post('/api/v1/execution/command', command).body,
      command,
    );
    expect(landed).toMatchObject({ applied: true, replayed: false });
    const template = JSON.parse(reviewTemplate(task)) as Record<
      string,
      unknown
    >;
    // 模板本身故意不完整（决定留给审阅人）：原样提交必须被拒，且不能弄坏之后的黑板读取。
    expect(
      post('/api/v1/execution/command', {
        operation: 'review',
        task_id: task.id,
        receipt: template,
      }).status,
    ).toBe(409);
    const receipt = {
      ...template,
      status: 'completed',
      acceptance: task.criteria.map((criterion) => ({
        criterion,
        result: 'pass',
        evidence: [],
      })),
      elapsed_ms: 9_000_000,
      tool_calls: 143,
      closeout: {
        learned: [],
        adr_required: false,
        rule_change_required: false,
        destinations: [],
      },
    };
    const reviewed = boardItem(
      post('/api/v1/execution/command', {
        operation: 'review',
        task_id: task.id,
        receipt,
      }).body,
      project,
    );
    expect(reviewed.status).toBe('succeeded');
    expect(reviewed.receipt?.status).toBe('completed');
    expect(
      boardItems(read(`/api/v1/board?${query({ project })}`), project),
    ).toHaveLength(7);
    const frozen = post('/api/v1/execution/command', {
      operation: 'freeze',
      frozen: true,
    });
    expect(frozen.body).toEqual({ frozen: true });
    expect(executionStatus(read('/api/v1/execution')).frozen).toBe(true);
  });

  it('编辑与文件操作：追平后保存、历史恢复、删除、从最近删除恢复、重命名', () => {
    const { get, post, read } = setup();
    const path = '项目/Studiolo/02-日志/草稿-未归档.md';
    const stale = canonicalDocument(
      read(`/api/v1/file?${query({ path })}`),
      path,
    );
    expect(stale.editing).toMatchObject({ allowed: false, reason: 'stale' });
    read('/api/workbench?refresh=true');
    const document = canonicalDocument(
      read(`/api/v1/file?${query({ path })}`),
      path,
    );
    expect(document.editing.allowed).toBe(true);
    const replacement = {
      kind: 'replace' as const,
      vault_uuid: document.vault_uuid,
      path,
      expected_revision: document.revision,
      content: `${document.content}\n补一行。\n`,
      idempotency_key: 'fixture-save-0001',
    };
    const saved = saveReceipt(
      post('/api/v1/mutations', replacement).body,
      replacement,
    );
    expect(post('/api/v1/mutations', replacement).body).toMatchObject({
      receipt: saved,
    });
    const scope = { vault_uuid: document.vault_uuid, path };
    const page = historyPage(
      read(`/api/v1/history?${query({ ...scope, limit: '20' })}`),
      scope,
      null,
    );
    expect(page.entries).toHaveLength(3);
    const oldest = page.entries[2];
    historyFile(
      read(
        `/api/v1/history/file?${query({ ...scope, revision: oldest.revision })}`,
      ),
      scope,
      oldest,
    );
    const restoration = {
      kind: 'restore' as const,
      vault_uuid: document.vault_uuid,
      path,
      expected_revision: saved.new_revision,
      source_revision: oldest.revision,
      idempotency_key: 'fixture-restore-0001',
    };
    saveReceipt(post('/api/v1/mutations', restoration).body, restoration);
    const capability = fileOperationCapability(read('/api/v1/file-operations'));
    expect(capability.rename_enabled).toBe(true);
    const current = canonicalDocument(
      read(`/api/v1/file?${query({ path })}`),
      path,
    );
    const deleting = {
      kind: 'delete' as const,
      vault_uuid: capability.vault_uuid,
      path,
      expected_revision: current.revision,
      idempotency_key: 'fixture-delete-0001',
    };
    fileOperationReceipt(
      post('/api/v1/file-operations', deleting).body,
      deleting,
    );
    expect(get(`/api/v1/file?${query({ path })}`).status).toBe(404);
    const tombstones = tombstonePage(
      read(
        `/api/v1/tombstones?${query({ vault_uuid: capability.vault_uuid, limit: '32' })}`,
      ),
      null,
    );
    const tombstone = tombstones.entries.find(
      (entry: TombstoneEntry) => entry.path === path,
    )!;
    const restoring = {
      kind: 'restore_tombstone' as const,
      vault_uuid: capability.vault_uuid,
      path,
      tombstone_id: tombstone.id,
      idempotency_key: 'fixture-undelete-0001',
    };
    fileOperationReceipt(
      post('/api/v1/file-operations', restoring).body,
      restoring,
      tombstone,
    );
    const restored = canonicalDocument(
      read(`/api/v1/file?${query({ path })}`),
      path,
    );
    const renaming = {
      kind: 'rename' as const,
      vault_uuid: capability.vault_uuid,
      from: path,
      to: '项目/Studiolo/02-日志/草稿-已整理.md',
      expected_revision: restored.revision,
      idempotency_key: 'fixture-rename-0001',
    };
    fileOperationReceipt(
      post('/api/v1/file-operations', renaming).body,
      renaming,
    );
    expect(workbench(read).generation).toBeGreaterThan(243);
  });

  it('退出后内容接口一律 401；重新登录拿到新的会话对', () => {
    const { get, post } = setup();
    expect(post('/api/v1/sessions/revoke', { scope: 'current' }).body).toEqual({
      revoked: 1,
    });
    expect(get('/api/workbench').status).toBe(401);
    expect(get('/api/v1/session').status).toBe(401);
    const login = post('/api/v1/sessions', {});
    expect(login.status).toBe(201);
    expect(sessionPair(login.body).session.state).toBe('active');
    expect(get('/api/workbench').status).toBe(200);
  });

  it('门禁：写接口缺 X-Studiolo-Request 或 CSRF 一律 403；读接口可直接查看；未知接口 404', () => {
    const { api, get } = setup();
    expect(
      api.handle({ method: 'GET', url: '/api/workbench', headers: {} }).status,
    ).toBe(200);
    expect(
      api.handle({
        method: 'POST',
        url: '/api/v1/policy/validate',
        headers: { 'x-studiolo-request': '1' },
        body: {},
      }).status,
    ).toBe(403);
    expect(
      api.handle({
        method: 'POST',
        url: '/api/v1/sessions/revoke',
        headers: { 'x-csrf-token': 'c2.0.0' },
        body: { scope: 'current' },
      }).status,
    ).toBe(403);
    expect(get('/api/v1/nope').status).toBe(404);
  });
});

describe('假数据插件：只在 vite serve 且 STUDIOLO_FIXTURES=1 时挂载', () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  function fakeServer() {
    return {
      middlewares: { use: vi.fn() },
      printUrls: vi.fn(),
      config: { logger: { warn: vi.fn() } },
    };
  }

  function configure(server: ReturnType<typeof fakeServer>) {
    const hook = studioloFixtures().configureServer;
    const handler = typeof hook === 'function' ? hook : hook?.handler;
    return handler?.call({} as never, server as never);
  }

  it('apply 固定为 serve：vite build 不会装上它', () => {
    expect(studioloFixtures().apply).toBe('serve');
  });

  it('没设 STUDIOLO_FIXTURES 时什么都不注册、不打印', async () => {
    vi.stubEnv(FIXTURE_ENV, '');
    vi.stubEnv('VITEST', '');
    const server = fakeServer();
    await configure(server);
    expect(server.middlewares.use).not.toHaveBeenCalled();
    server.printUrls();
    expect(server.config.logger.warn).not.toHaveBeenCalled();
    expect(fixturesEnabled({ [FIXTURE_ENV]: 'true' })).toBe(false);
    expect(fixturesEnabled({ [FIXTURE_ENV]: '1', VITEST: 'true' })).toBe(false);
  });

  it('STUDIOLO_FIXTURES=1 时在代理之前应答 /api/*，其余请求放行，并打印一行提示', async () => {
    vi.stubEnv(FIXTURE_ENV, '1');
    vi.stubEnv('VITEST', '');
    const server = fakeServer();
    const original = server.printUrls;
    await configure(server);
    expect(server.middlewares.use).toHaveBeenCalledTimes(1);
    const middleware = server.middlewares.use.mock.calls[0][0] as (
      request: unknown,
      response: unknown,
      next: () => void,
    ) => void;
    const next = vi.fn();
    middleware(
      Object.assign(new PassThrough(), { url: '/src/main.tsx' }),
      {},
      next,
    );
    expect(next).toHaveBeenCalledTimes(1);
    const request = Object.assign(new PassThrough(), {
      url: '/api/v1/health',
      method: 'GET',
      headers: { 'x-studiolo-request': '1' },
    });
    const response = {
      statusCode: 0,
      headers: {} as Record<string, string>,
      setHeader(name: string, value: string) {
        this.headers[name.toLowerCase()] = value;
      },
      end: vi.fn(),
    };
    middleware(request, response, next);
    request.end();
    await vi.waitFor(() => expect(response.end).toHaveBeenCalled());
    expect(next).toHaveBeenCalledTimes(1);
    expect(response.statusCode).toBe(200);
    expect(response.headers['x-studiolo-fixture']).toBe('exec-0922');
    expect(
      parseHealth(JSON.parse(String(response.end.mock.calls[0][0]))).overall,
    ).toBe('degraded');
    server.printUrls();
    expect(original).toHaveBeenCalledTimes(1);
    expect(server.config.logger.warn).toHaveBeenCalledTimes(1);
    expect(String(server.config.logger.warn.mock.calls[0][0])).toContain(
      '假数据模式',
    );
  });
});
