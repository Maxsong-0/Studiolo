/**
 * 假数据模式的 `/api/*` 应答器：纯函数式入口 `handle(request)`，不碰网络与文件系统，
 * 时钟可注入——Vite 插件拿它接请求，`src/test/devFixtures.test.ts` 拿它把每份假数据喂给真实解码器。
 *
 * 行为贴着真实接口：会话模式（带 CSRF 与主人范围）；首屏「同步偏慢」，点刷新（`?refresh=true`）
 * 后追平；发布策略、落地候选、提交审阅、解冻执行都要先过二次认证（验证码任意六位数字，000000 视为
 * 未通过）；写操作改的是内存里的状态，重启开发服务器即复原。未知接口一律 404。
 */
import { DAY, HOUR, iso } from './fixtures/clock.ts';
import {
  FINISHED,
  TASK_BY_KEY,
  TASK_DEFS,
  boardWire,
  initialTaskState,
  overviewItem,
  patchSha256,
  taskKeyOf,
} from './fixtures/board.ts';
import type { Receipt, TaskState } from './fixtures/board.ts';
import { documentView, search, treePage } from './fixtures/documents.ts';
import {
  appendEvents,
  humanEvents,
  initialEvents,
  replayEvent,
  timelineEvent,
} from './fixtures/events.ts';
import type { BoardEvent } from './fixtures/events.ts';
import { L3_REQUIRED, initialRuns, runWire } from './fixtures/execution.ts';
import type { RunRecord } from './fixtures/execution.ts';
import { VaultStore } from './fixtures/files.ts';
import type { Outcome } from './fixtures/files.ts';
import { healthBody } from './fixtures/health.ts';
import {
  ATTEMPT,
  CSRF_TOKEN,
  SESSION_ID,
  TASK,
  VAULT_UUID,
  sha256,
} from './fixtures/ids.ts';
import type { TaskKey } from './fixtures/ids.ts';
import { projectOperations } from './fixtures/operations.ts';
import {
  activeRevocations,
  effectiveEntries,
  globalRevoked,
  initialPolicy,
  pendingCount,
  publish,
  validate,
} from './fixtures/policy.ts';
import type { DesiredRecord, Kind, PolicyState } from './fixtures/policy.ts';
import {
  REJECTED_CODE,
  STEP_UP_WINDOW_MS,
  factorState,
  initialSessions,
  recoveryCodes,
  sessionInfo,
  sessionPair,
} from './fixtures/session.ts';
import type { SessionRecord } from './fixtures/session.ts';
import {
  ATTACHMENTS,
  PROJECT_NAMES,
  buildVault,
  root,
} from './fixtures/vault.ts';
import {
  SOURCE_LABEL,
  deriveWorkbench,
  freshness,
  known,
  rollup,
} from './fixtures/workbench.ts';

export interface FixtureRequest {
  method: string;
  /** 路径加查询串，例如 `/api/v1/board?project=%E9%A1%B9…`。 */
  url: string;
  /** 小写头名。 */
  headers: Record<string, string | undefined>;
  /** 已解析的 JSON 正文；GET 为 undefined。 */
  body?: unknown;
}
export type FixtureResponse = Outcome;

const ok = (body: unknown): Outcome => ({ status: 200, body });
const error = (status: number, message: string, code?: string): Outcome => ({
  status,
  body: code ? { code, error: message } : { error: message },
});
const record = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);
const PROJECT_ROOTS = PROJECT_NAMES.map(root);
const STALE_VIEW = '视图已刷新，请刷新工作台后重试。';

/** 只允许这些查询参数（后端 `deny_unknown_fields` 的同一口径）。 */
function only(query: URLSearchParams, allowed: string[]) {
  return [...query.keys()].every((key) => allowed.includes(key));
}

function counter(value: string | null): number | null {
  return value !== null && /^(0|[1-9]\d{0,15})$/.test(value)
    ? Number(value)
    : null;
}

export function createFixtureApi(options: { now?: () => number } = {}) {
  const clock = options.now ?? Date.now;
  const start = clock();
  const notes = buildVault(start);
  let generation = 243;
  let committed = 48_216;
  let fresh = false;
  const vault = new VaultStore(notes, start, () => {
    generation += 1;
    committed += 1;
  });
  let sessions: SessionRecord[] = initialSessions(start);
  let current: string | null = SESSION_ID;
  let recent: { verified_at_ms: number; expires_at_ms: number } | null = null;
  const tasks = new Map<TaskKey, TaskState>(
    TASK_DEFS.map((def) => [def.key, initialTaskState(def)]),
  );
  let events: BoardEvent[] = initialEvents(start);
  let runs: RunRecord[] = initialRuns();
  let frozen = false;
  const policy: PolicyState = initialPolicy(start);

  const stepUpValid = (now: number) =>
    recent !== null && recent.expires_at_ms > now;
  const currentFreshness = () => freshness(fresh, committed);

  // ---------- 首屏 ----------
  function workbench() {
    const parts = deriveWorkbench(notes);
    const projects = parts.projects as {
      id: string;
      active_tasks: { value: number | null };
      open_bugs: { value: number | null };
    }[];
    return {
      generation,
      source_label: SOURCE_LABEL,
      freshness: currentFreshness(),
      projects: parts.projects,
      inbox: parts.inbox,
      diagnostics: [],
      counts: {
        documents: known(parts.documents),
        projects: known(projects.length),
        inbox: known(parts.inbox.length),
        diagnostics: known(0),
        active_tasks: rollup(
          projects.map((project) => [project.id, project.active_tasks.value]),
        ),
        open_bugs: rollup(
          projects.map((project) => [project.id, project.open_bugs.value]),
        ),
      },
    };
  }

  // ---------- 黑板 ----------
  const time = (now: number) => ({ now, anchor: start });
  const wire = (key: TaskKey, now: number) =>
    boardWire(
      TASK_BY_KEY.get(key)!,
      tasks.get(key)!,
      time(now),
      policy.effective,
    );

  function overview(query: URLSearchParams, now: number): Outcome {
    if (!only(query, ['window_hours']))
      return error(400, '只接受 window_hours 参数。');
    const raw = query.get('window_hours');
    const hours = raw === null ? 24 : counter(raw);
    if (hours === null || hours < 1 || hours > 168)
      return error(400, 'window_hours 必须是 1 到 168 的整数。');
    const from = now - hours * HOUR;
    const windowed = events
      .filter((event) => event.at >= from && event.at <= now)
      .slice(-1000);
    const items = TASK_DEFS.filter(
      (def) =>
        !FINISHED.includes(tasks.get(def.key)!.status) ||
        windowed.some((event) => event.task === def.key),
    )
      .sort((left, right) =>
        left.project === right.project
          ? TASK[left.key] < TASK[right.key]
            ? -1
            : 1
          : left.project < right.project
            ? -1
            : 1,
      )
      .map((def) => overviewItem(def, tasks.get(def.key)!, time(now)));
    return ok({
      generated_at: iso(now),
      window_from: iso(from),
      window_to: iso(now),
      policy_hash: policy.effective,
      items,
      items_truncated: false,
      events: windowed.map((event) => ({
        at: iso(event.at),
        project: TASK_BY_KEY.get(event.task)!.project,
        task_id: TASK[event.task],
        action: event.action,
        phase: event.phase,
        attempt_id: event.attemptId,
      })),
      events_truncated: false,
    });
  }

  function timeline(query: URLSearchParams): Outcome {
    if (!only(query, ['project', 'from', 'to', 'after']))
      return error(400, '查询参数无效。');
    const project = query.get('project') ?? '';
    const from = query.get('from') ?? '';
    const to = query.get('to') ?? '';
    const start = Date.parse(from);
    const end = Date.parse(to);
    const after = query.has('after') ? counter(query.get('after')) : 0;
    if (
      !project.startsWith('项目/') ||
      project.split('/').length !== 2 ||
      !Number.isFinite(start) ||
      !Number.isFinite(end) ||
      start > end ||
      end - start > 31 * DAY ||
      after === null
    )
      return error(400, '回放范围无效。');
    const matching = events.filter(
      (event) =>
        TASK_BY_KEY.get(event.task)!.project === project &&
        event.at >= start &&
        event.at <= end &&
        Number(event.offset) > after,
    );
    const prefix = `${project}/02-日志/`;
    const logs: { path: string; title: string; date: string }[] = [];
    let undated = 0;
    for (const note of notes) {
      if (!note.path.startsWith(prefix)) continue;
      const date = note.path.slice(prefix.length, prefix.length + 10);
      if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) {
        undated += 1;
        continue;
      }
      if (date >= from.slice(0, 10) && date <= to.slice(0, 10))
        logs.push({
          path: note.path,
          title: documentView(notes, note.path)!.title,
          date,
        });
    }
    return ok({
      project,
      from,
      to,
      events: matching.slice(0, 1000).map(timelineEvent),
      logs,
      undated_logs: undated,
      next: matching.length > 1000 ? matching[999].offset : null,
    });
  }

  function replay(query: URLSearchParams, now: number): Outcome {
    if (!only(query, ['task_id', 'after'])) return error(400, '查询参数无效。');
    const key = taskKeyOf(query.get('task_id') ?? '');
    const after = query.has('after') ? counter(query.get('after')) : 0;
    if (after === null) return error(400, '回放游标无效。');
    if (!key) return error(503, '工作项不存在');
    const own = events.filter(
      (event) => event.task === key && Number(event.offset) > after,
    );
    return ok({
      task: wire(key, now),
      events: own.slice(0, 100).map(replayEvent),
      next: own.length > 100 ? own[99].offset : null,
    });
  }

  // ---------- 执行 ----------
  function execute(body: Record<string, unknown>, now: number): Outcome {
    const operation = body.operation;
    const key =
      typeof body.task_id === 'string' ? taskKeyOf(body.task_id) : undefined;
    const log = (action: string, outcome: Record<string, string>) => {
      if (key)
        events = appendEvents(events, humanEvents(key, now, action, outcome));
    };
    if (operation === 'freeze') {
      if (body.frozen === false && !stepUpValid(now))
        return error(409, L3_REQUIRED);
      frozen = body.frozen === true;
      return ok({ frozen });
    }
    if (!key) return error(409, '工作项不存在或不在本入口登记。');
    const task = tasks.get(key)!;
    const run = runs.find((item) => item.key === key);
    if (operation === 'start')
      return error(409, '任务不在待认领状态，不能启动执行体。');
    if (operation === 'cancel') {
      if (!run || run.state !== 'running' || task.status !== 'running')
        return error(409, '该任务没有在跑的执行体。');
      run.state = 'stopped';
      task.status = 'cancel_requested';
      log('execution.cancel', {});
      return ok({ ok: true });
    }
    if (operation === 'land') {
      if (!stepUpValid(now)) return error(409, L3_REQUIRED);
      if (task.status !== 'needs_review' || task.receipt)
        return error(409, '只有已封存、待审阅的代码候选可以落地。');
      if (body.patch_sha256 !== patchSha256(key))
        return error(409, '候选 SHA-256 与封存指针不一致。');
      if (
        typeof body.expected_head !== 'string' ||
        !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(body.expected_head)
      )
        return error(400, '目标 HEAD 格式无效。');
      const replayed = task.landed;
      task.landed = true;
      log('execution.land', { status: 'applied' });
      return ok({
        task_id: TASK[key],
        attempt_id: ATTEMPT[key],
        patch_sha256: body.patch_sha256,
        expected_head: body.expected_head,
        after_tree: sha256(`tree ${key} ${body.expected_head}`).slice(0, 40),
        applied: true,
        replayed,
        conflicts: [],
      });
    }
    if (operation === 'review') {
      if (!stepUpValid(now)) return error(409, L3_REQUIRED);
      const def = TASK_BY_KEY.get(key)!;
      const receipt = body.receipt;
      if (!['needs_review', 'approved', 'needs_revision'].includes(task.status))
        return error(409, '该任务当前不能提交审阅。');
      if (!validReceipt(receipt, def.criteria))
        return error(409, '回执字段不完整或不符合任务验收项。');
      if (
        receipt.status === 'completed' &&
        def.candidate === 'patch' &&
        !task.landed
      )
        return error(409, '代码候选尚未落地，不能以 completed 收尾。');
      task.status = {
        completed: 'succeeded',
        failed: 'failed',
        stopped: 'cancelled',
        abandoned: 'abandoned',
      }[receipt.status] as TaskState['status'];
      task.phase =
        receipt.status === 'completed'
          ? 'completed'
          : receipt.status === 'stopped'
            ? 'cancelled'
            : (receipt.status as TaskState['phase']);
      task.receipt = receipt;
      runs = runs.filter((item) => item.key !== key);
      log('execution.review', { status: task.status });
      return ok(wire(key, now));
    }
    return error(400, '请求格式无效。');
  }

  // ---------- 策略 ----------
  function policyStatus() {
    return {
      effective_hash: policy.effective,
      entries: effectiveEntries(policy),
      global_revoked: globalRevoked(policy),
      pending_desired: pendingCount(policy),
      revocations: activeRevocations(policy),
      l3_available: true,
    };
  }

  const desiredOf = (id: unknown) =>
    policy.desired.find((item) => item.id === id);

  function policyPost(
    path: string,
    body: Record<string, unknown>,
    now: number,
  ): Outcome {
    if (path === '/api/v1/policy/desired') {
      const kind = body.kind as Kind;
      if (
        !['rules', 'skills', 'capability_set'].includes(String(kind)) ||
        typeof body.subject !== 'string' ||
        !(body.body === null || typeof body.body === 'string')
      )
        return error(400, '请求格式无效。', 'invalid');
      const { report } = validate(policy, kind, body.subject, body.body);
      const desired: DesiredRecord = {
        id: newUlid(now),
        kind,
        subject: body.subject,
        body: body.body,
        body_sha256: report.body_sha256,
        submitter_kind: 'owner',
        agent_type: null,
        agent_instance: null,
        submitted_at: iso(now),
        status: report.ok ? 'validated' : 'rejected',
        report,
        validated_at: iso(now),
        activated_version: null,
      };
      policy.desired.unshift(desired);
      return ok({ desired });
    }
    if (
      path === '/api/v1/policy/validate' ||
      path === '/api/v1/policy/withdraw'
    ) {
      const desired = desiredOf(body.desired_id);
      if (!desired) return error(404, 'desired 不存在', 'not_found');
      if (!['pending', 'validated', 'rejected'].includes(desired.status))
        return error(
          409,
          `desired 当前状态 ${desired.status} 不可更改`,
          'state',
        );
      if (path.endsWith('withdraw')) desired.status = 'withdrawn';
      else {
        const { report } = validate(
          policy,
          desired.kind,
          desired.subject,
          desired.body,
        );
        desired.report = report;
        desired.status = report.ok ? 'validated' : 'rejected';
        desired.validated_at = iso(now);
      }
      return ok({ desired });
    }
    if (path === '/api/v1/policy/activate') {
      if (!stepUpValid(now))
        return error(
          403,
          '发布策略需要先完成二次认证（One-time Code）。',
          'reauthentication_required',
        );
      if (body.expected_effective_hash !== policy.effective)
        return {
          status: 409,
          body: {
            code: 'hash_mismatch',
            error: 'effective 已变化，请重新校验后再发布。',
            current_effective_hash: policy.effective,
          },
        };
      if (body.release_global_revocation === true) {
        if (!globalRevoked(policy))
          return error(409, '当前没有全局吊销', 'state');
        for (const row of activeRevocations(policy))
          if (row.level === 'global') {
            row.released_at = iso(now);
            row.released_by = 'owner';
          }
        return ok({
          released: true,
          effective_hash: policy.effective,
          audit_confirmed: true,
        });
      }
      if (typeof body.rollback_to === 'number') {
        const target = policy.versions.find(
          (row) => row.meta.version === body.rollback_to,
        );
        if (!target) return error(404, 'version not found', 'not_found');
        const { report } = validate(
          policy,
          target.meta.kind,
          target.meta.subject,
          target.body,
        );
        if (!report.ok)
          return error(
            422,
            '校验未通过；逐条问题见该 desired 的报告。',
            'validation_failed',
          );
        const activation = publish(
          policy,
          {
            kind: target.meta.kind,
            subject: target.meta.subject,
            body: target.body,
            report,
            desiredId: null,
            rollbackOf: target.meta.version,
          },
          now,
        );
        return ok({ activation, audit_confirmed: true });
      }
      const desired = desiredOf(body.desired_id);
      if (!desired) return error(404, 'desired 不存在', 'not_found');
      if (!['pending', 'validated', 'rejected'].includes(desired.status))
        return error(
          409,
          `desired 当前状态 ${desired.status} 不可发布`,
          'state',
        );
      const { report } = validate(
        policy,
        desired.kind,
        desired.subject,
        desired.body,
      );
      if (!report.ok) {
        desired.status = 'rejected';
        desired.report = report;
        return error(
          422,
          '校验未通过；逐条问题见该 desired 的报告。',
          'validation_failed',
        );
      }
      const activation = publish(
        policy,
        {
          kind: desired.kind,
          subject: desired.subject,
          body: desired.body,
          report,
          desiredId: desired.id,
          rollbackOf: null,
        },
        now,
      );
      desired.status = 'activated';
      desired.report = report;
      desired.activated_version = activation.version;
      for (const other of policy.desired)
        if (
          other !== desired &&
          other.kind === desired.kind &&
          other.subject === desired.subject &&
          ['pending', 'validated', 'rejected'].includes(other.status)
        )
          other.status = 'superseded';
      return ok({ activation, audit_confirmed: true });
    }
    if (path === '/api/v1/agents/revoke') {
      const level = body.level;
      if (
        (level !== 'instance' && level !== 'type' && level !== 'global') ||
        typeof body.reason !== 'string' ||
        !body.reason.trim()
      )
        return error(400, '请求格式无效。', 'invalid');
      const target = level === 'global' ? '*' : String(body.target ?? '');
      if (!target) return error(400, '吊销目标不能为空。', 'invalid');
      const existing = activeRevocations(policy).some(
        (row) => row.level === level && row.target === target,
      );
      if (!existing)
        policy.revocations.push({
          seq: Math.max(...policy.revocations.map((row) => row.seq)) + 1,
          level,
          target,
          revoked_at: iso(now),
          revoked_by: 'owner',
          reason: body.reason,
          released_at: null,
          released_by: null,
        });
      return ok({ created: !existing, level, target });
    }
    return error(404, `假数据模式没有这个接口：POST ${path}`);
  }

  // ---------- 会话 ----------
  function sessionPost(path: string, body: unknown, now: number): Outcome {
    if (path === '/api/v1/sessions') {
      if (current) return error(409, '已有应用 Cookie，不能隐式换发会话。');
      const created: SessionRecord = {
        id: `v2-${sha256(`session ${now}`).slice(0, 16)}`,
        created_at_ms: now,
        last_seen_at_ms: now,
      };
      sessions = [created, ...sessions];
      current = created.id;
      return { status: 201, body: sessionPair(created) };
    }
    if (path === '/api/v1/sessions/revoke') {
      if (!record(body)) return error(400, '认证请求格式无效。');
      let removed: SessionRecord[] = [];
      if (body.scope === 'current')
        removed = sessions.filter((item) => item.id === current);
      else if (body.scope === 'all') removed = sessions;
      else if (body.scope === 'session')
        removed = sessions.filter((item) => item.id === body.session_id);
      else return error(400, '认证请求格式无效。');
      sessions = sessions.filter((item) => !removed.includes(item));
      if (!sessions.some((item) => item.id === current)) current = null;
      return ok({ revoked: removed.length });
    }
    if (path === '/api/v1/auth/step-up') {
      if (
        !record(body) ||
        typeof body.code !== 'string' ||
        !/^\d{6}$/.test(body.code)
      )
        return error(400, '认证请求格式无效。');
      if (body.code === REJECTED_CODE) return error(403, '验证或请求未通过。');
      recent = { verified_at_ms: now, expires_at_ms: now + STEP_UP_WINDOW_MS };
      return ok(recent);
    }
    if (path === '/api/v1/auth/recovery-codes') {
      if (!stepUpValid(now)) return error(403, '需要近期二次认证。');
      return ok(recoveryCodes(now));
    }
    if (path === '/api/v1/auth/factor/recover') {
      sessions = [];
      current = null;
      recent = null;
      return { status: 204 };
    }
    return error(404, `假数据模式没有这个接口：POST ${path}`);
  }

  // ---------- 路由 ----------
  function get(pathname: string, query: URLSearchParams, now: number): Outcome {
    const session = sessions.find((item) => item.id === current)!;
    switch (pathname) {
      case '/api/v1/session':
        return ok(sessionPair(session));
      case '/api/v1/sessions':
        return ok({ sessions: sessions.map(sessionInfo) });
      case '/api/v1/auth/state':
        return ok(factorState(now, recent));
      case '/api/workbench':
        if (!only(query, ['refresh'])) return error(400, '查询参数无效。');
        if (query.get('refresh') === 'true') fresh = true;
        return ok(workbench());
      case '/api/document': {
        if (counter(query.get('generation')) !== generation)
          return error(409, STALE_VIEW);
        const view = documentView(notes, query.get('path') ?? '');
        return view ? ok(view) : error(404, '文档不在当前只读快照中。');
      }
      case '/api/search': {
        const mode = query.get('mode');
        const text = query.get('q') ?? '';
        const project = query.get('project');
        // 与 crates/workbench/src/search.rs 的 validate 同一口径：空查询合法（路径模式列举、全文模式空结果），
        // 只拒绝超长（256 字符或 1024 字节）与含控制字符的查询。
        if (
          (mode !== 'full_text' && mode !== 'path') ||
          query.get('q') === null ||
          [...text].length > 256 ||
          new TextEncoder().encode(text).length > 1024 ||
          /\p{Cc}/u.test(text)
        )
          return error(400, '查询须为不含控制字符的短文本（最多 256 字符）。');
        if (project !== null && !PROJECT_ROOTS.includes(project))
          return error(400, '项目不在当前只读快照中。');
        if (counter(query.get('generation')) !== generation)
          return error(409, STALE_VIEW);
        return ok(search(notes, text, mode, project, generation));
      }
      case '/api/tree': {
        if (counter(query.get('generation')) !== generation)
          return error(409, STALE_VIEW);
        const offset = counter(query.get('offset'));
        const page =
          offset === null
            ? null
            : treePage(
                notes,
                ATTACHMENTS,
                query.get('path') ?? '',
                offset,
                generation,
              );
        if (!page) return error(404, '目录不在当前只读快照中。');
        if (offset !== 0 && offset! >= page.total)
          return error(400, '分页位置越界。');
        return ok(page);
      }
      case '/api/project-operations': {
        if (!only(query, ['project', 'generation']))
          return error(400, '查询参数无效。');
        if (counter(query.get('generation')) !== generation)
          return error(409, STALE_VIEW);
        const project = query.get('project') ?? '';
        return PROJECT_ROOTS.includes(project)
          ? ok(projectOperations(project, generation, notes, now))
          : error(404, '项目不在当前只读快照中。');
      }
      case '/api/v1/file': {
        const document = vault.canonical(
          query.get('path') ?? '',
          currentFreshness(),
        );
        return document
          ? ok(document)
          : error(404, '文档不存在。', 'not_found');
      }
      case '/api/v1/history': {
        const page = vault.historyPage(
          query.get('vault_uuid'),
          query.get('path') ?? '',
        );
        return page ? ok(page) : error(404, '没有这份文档的页面历史。');
      }
      case '/api/v1/history/file': {
        const file = vault.historyFile(
          query.get('vault_uuid'),
          query.get('path') ?? '',
          query.get('revision'),
        );
        return file ? ok(file) : error(404, '历史版本不存在。');
      }
      case '/api/v1/file-operations':
        return ok({
          vault_uuid: VAULT_UUID,
          enabled: true,
          rename_enabled: true,
        });
      case '/api/v1/tombstones':
        return query.get('vault_uuid') === VAULT_UUID
          ? ok(vault.tombstonePage())
          : error(404, '没有这个 Vault 的删除记录。');
      case '/api/v1/health':
        return ok(healthBody());
      case '/api/v1/board': {
        const project = query.get('project');
        if (
          !only(query, ['project']) ||
          !project ||
          project.split('/').some((part) => !part)
        )
          return error(400, '查询参数无效。');
        return ok(
          TASK_DEFS.filter((def) => def.project === project)
            .sort((left, right) => (TASK[left.key] < TASK[right.key] ? -1 : 1))
            .map((def) => wire(def.key, now)),
        );
      }
      case '/api/v1/board/overview':
        return overview(query, now);
      case '/api/v1/board/timeline':
        return timeline(query);
      case '/api/v1/board/replay':
        return replay(query, now);
      case '/api/v1/execution':
        return ok({ configured: true, frozen, runs: runs.map(runWire) });
      case '/api/v1/policy':
        return ok(policyStatus());
      case '/api/v1/policy/desired':
        return ok({ desired: policy.desired });
      case '/api/v1/policy/versions':
        return ok({
          versions: [...policy.versions].reverse().map((row) => row.meta),
        });
    }
    const version = /^\/api\/v1\/policy\/versions\/(\d+)$/.exec(pathname);
    if (version) {
      const found = policy.versions.find(
        (row) => row.meta.version === Number(version[1]),
      );
      return found
        ? ok({ version: found })
        : error(404, 'version not found', 'not_found');
    }
    return error(404, `假数据模式没有这个接口：GET ${pathname}`);
  }

  function handle(request: FixtureRequest): Outcome {
    const now = clock();
    const url = new URL(request.url, 'http://fixture.invalid');
    const { pathname } = url;
    const query = url.searchParams;
    if (pathname === '/api/v1/bootstrap')
      return request.method === 'GET'
        ? ok({ authentication: 'session' })
        : error(405, '此认证入口不接受该方法。');
    const login = request.method === 'POST' && pathname === '/api/v1/sessions';
    const session = sessions.find((item) => item.id === current);
    if (!login && !session) return error(401, '需要有效的应用会话。');
    if (session) session.last_seen_at_ms = now;
    // 读接口不查请求头，方便直接 curl 查看假数据；写接口与真实入口一样要求
    // X-Studiolo-Request 与 CSRF（登录本身除外）。
    if (request.method === 'GET') return get(pathname, query, now);
    if (request.method !== 'POST') return error(405, '此入口不接受该方法。');
    if (
      request.headers['x-studiolo-request'] !== '1' ||
      (!login && request.headers['x-csrf-token'] !== CSRF_TOKEN)
    )
      return error(403, '认证请求不符合安全策略。');
    const body = request.body;
    if (
      pathname.startsWith('/api/v1/sessions') ||
      pathname.startsWith('/api/v1/auth/')
    )
      return sessionPost(pathname, body, now);
    if (!record(body)) return error(400, '请求格式无效。');
    if (pathname === '/api/v1/mutations')
      return vault.mutate(body, now, !fresh);
    if (pathname === '/api/v1/file-operations')
      return vault.fileOperation(body, now);
    if (pathname === '/api/v1/execution/command') return execute(body, now);
    if (
      pathname.startsWith('/api/v1/policy/') ||
      pathname === '/api/v1/agents/revoke'
    )
      return policyPost(pathname, body, now);
    return error(404, `假数据模式没有这个接口：POST ${pathname}`);
  }

  return { handle };
}

const exactly = (
  value: unknown,
  keys: string,
): value is Record<string, unknown> =>
  record(value) &&
  Object.keys(value).length === keys.split(' ').length &&
  keys.split(' ').every((key) => Object.hasOwn(value, key));
const texts = (value: unknown) =>
  Array.isArray(value) &&
  value.every((item) => typeof item === 'string' && item.trim() !== '');
const logicalPath = (value: unknown) =>
  typeof value === 'string' &&
  value !== '' &&
  !value.includes('\\') &&
  value.split('/').every((part) => part && part !== '.' && part !== '..');
const pointers = (value: unknown) =>
  Array.isArray(value) &&
  value.every(
    (item) =>
      exactly(item, 'path revision sha256') &&
      logicalPath(item.path) &&
      typeof item.revision === 'string' &&
      item.revision.trim() !== '' &&
      item.revision.length <= 256 &&
      /^[a-fA-F0-9]{64}$/.test(String(item.sha256)),
  );
const count = (value: unknown) =>
  Number.isSafeInteger(value) && (value as number) >= 0;

/**
 * 回执与真实后端一样封闭校验（字段集合、指针形状、逐条对应验收项、completed 必须全部通过）：
 * 存进去的回执之后会出现在黑板读取里，不合格的一条就会让整页解码失败。
 */
function validReceipt(value: unknown, criteria: string[]): value is Receipt {
  if (
    !exactly(
      value,
      'status artifacts conflicts unlanded acceptance remaining elapsed_ms tool_calls learned closeout',
    )
  )
    return false;
  const acceptance = value.acceptance;
  const closeout = value.closeout;
  if (
    !['completed', 'failed', 'stopped', 'abandoned'].includes(
      String(value.status),
    ) ||
    !Array.isArray(acceptance) ||
    acceptance.length !== criteria.length ||
    acceptance.some(
      (item, index) =>
        !exactly(item, 'criterion result evidence') ||
        item.criterion !== criteria[index] ||
        !['pass', 'fail', 'unverified'].includes(String(item.result)) ||
        !pointers(item.evidence),
    ) ||
    !pointers(value.artifacts) ||
    !Array.isArray(value.conflicts) ||
    value.conflicts.length > 0 ||
    !Array.isArray(value.unlanded) ||
    !value.unlanded.every(
      (item) =>
        exactly(item, 'path sha256') &&
        logicalPath(item.path) &&
        /^[a-fA-F0-9]{64}$/.test(String(item.sha256)),
    ) ||
    !texts(value.remaining) ||
    !texts(value.learned) ||
    !count(value.elapsed_ms) ||
    !count(value.tool_calls) ||
    !exactly(
      closeout,
      'learned adr_required rule_change_required destinations',
    ) ||
    !texts(closeout.learned) ||
    typeof closeout.adr_required !== 'boolean' ||
    typeof closeout.rule_change_required !== 'boolean' ||
    !Array.isArray(closeout.destinations) ||
    !closeout.destinations.every(logicalPath)
  )
    return false;
  return (
    value.status !== 'completed' ||
    (acceptance.every(
      (item) => (item as Record<string, unknown>).result === 'pass',
    ) &&
      value.unlanded.length === 0 &&
      (value.remaining as unknown[]).length === 0)
  );
}

const ULID_ALPHABET = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
let ulidSequence = 0;
/** 新 desired 的 ULID：时间戳部分取此刻，随机部分用计数器派生，保证同毫秒也不重复。 */
function newUlid(now: number) {
  let time = now;
  let head = '';
  for (let index = 0; index < 10; index += 1) {
    head = ULID_ALPHABET[time % 32] + head;
    time = Math.floor(time / 32);
  }
  ulidSequence += 1;
  const digest = sha256(`desired ${now} ${ulidSequence}`);
  let tail = '';
  for (let index = 0; index < 16; index += 1)
    tail +=
      ULID_ALPHABET[parseInt(digest.slice(index * 2, index * 2 + 2), 16) % 32];
  return head + tail;
}
