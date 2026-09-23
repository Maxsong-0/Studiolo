import { vi, type Mock } from 'vitest';
import { installPolicyWriter } from '../../policyApi';
import { ApiError } from '../../sessionHttp';
import { jsonResponse } from '../fixtures';

/** 策略平面界面测试的共用夹具：读走 fetch，写走 installPolicyWriter（与 policyPanel.test 同一做法）。 */

export const HASH = 'b'.repeat(64);
export const DESIRED_ID = '01ABCDEFGHJKMNPQRSTVWXYZ00';

type Body = Record<string, unknown>;

export function statusBody(overrides: Body = {}): Body {
  return {
    effective_hash: HASH,
    entries: [
      {
        kind: 'capability_set',
        subject: 'claude',
        version: 2,
        body_sha256: HASH,
        activated_at: '2026-09-17T00:00:00Z',
      },
    ],
    global_revoked: false,
    pending_desired: 1,
    revocations: [],
    l3_available: true,
    ...overrides,
  };
}

export function reportBody(overrides: Body = {}): Body {
  return {
    ok: true,
    kind: 'capability_set',
    subject: 'claude',
    retire: false,
    problems: [],
    diff: {
      format: 'json',
      entries: [
        {
          op: 'added',
          path: 'read.project_whitelist[1]',
          before: null,
          after: '项目/Semiment',
        },
        {
          op: 'removed',
          path: 'read.project_whitelist[0]',
          before: '项目/旧项目',
          after: null,
        },
      ],
      added: 1,
      removed: 1,
      changed: 0,
      truncated: false,
    },
    expansions: [
      {
        path: 'read.project_whitelist',
        before: null,
        after: '项目/Semiment',
        note: '读取范围变宽',
      },
    ],
    narrowings: [
      {
        path: 'read.project_whitelist',
        before: '项目/旧项目',
        after: null,
        note: '读取范围收窄',
      },
    ],
    base_effective_hash: HASH,
    current_version: 2,
    body_sha256: HASH,
    ...overrides,
  };
}

export function desiredBody(overrides: Body = {}): Body {
  return {
    id: DESIRED_ID,
    kind: 'capability_set',
    subject: 'claude',
    body: '{}',
    body_sha256: HASH,
    submitter_kind: 'owner',
    agent_type: null,
    agent_instance: null,
    submitted_at: '2026-09-17T00:00:00Z',
    status: 'validated',
    report: reportBody(),
    validated_at: '2026-09-17T00:00:00Z',
    activated_version: null,
    ...overrides,
  };
}

export function versionBody(version: number, overrides: Body = {}): Body {
  return {
    version,
    kind: 'capability_set',
    subject: 'claude',
    retired: false,
    body_sha256: HASH,
    effective_hash: HASH,
    previous_effective_hash: HASH,
    activated_at: '2026-09-17T00:00:00Z',
    activated_by: 'owner',
    desired_id: null,
    rollback_of: null,
    expansions: [],
    ...overrides,
  };
}

export interface PolicyServerOptions {
  status?: Body;
  desired?: Body[];
  versions?: Body[];
  /** 写接口的应答；缺省发布一律要二次认证、撤销接入一律成功。 */
  write?: (path: string, body: unknown) => unknown;
}

export interface PolicyServer {
  fetchMock: Mock<typeof fetch>;
  posts: { path: string; body: unknown }[];
  /** 已发出的读取路径，按顺序。 */
  reads: () => string[];
  uninstall: () => void;
}

/** 装一个假的策略平面：GET 走 fetch，写走受控会话写者；返回请求记录供断言。 */
export function policyServer(options: PolicyServerOptions = {}): PolicyServer {
  const status = options.status ?? statusBody();
  const desired = options.desired ?? [];
  const versions = options.versions ?? [];
  const fetchMock = vi.fn<typeof fetch>(async (input) => {
    const path = String(input);
    if (path === '/api/v1/policy') return jsonResponse(status);
    if (path === '/api/v1/policy/desired') return jsonResponse({ desired });
    if (path === '/api/v1/policy/versions') return jsonResponse({ versions });
    const match = /^\/api\/v1\/policy\/versions\/(\d+)$/.exec(path);
    const meta = match
      ? versions.find((row) => String(row.version) === match[1])
      : undefined;
    if (meta)
      return jsonResponse({
        version: { meta, body: `v${String(meta.version)} 正文` },
      });
    return jsonResponse({ error: '未知' }, 404);
  });
  vi.stubGlobal('fetch', fetchMock);
  const posts: { path: string; body: unknown }[] = [];
  const write =
    options.write ??
    ((path: string) => {
      if (path === '/api/v1/policy/activate')
        throw new ApiError('发布策略需要先完成二次认证。', 403, {
          code: 'reauthentication_required',
        });
      if (path === '/api/v1/agents/revoke')
        return { created: true, level: 'instance', target: 'x' };
      throw new ApiError('未知', 404);
    });
  const uninstall = installPolicyWriter(async (path, body) => {
    posts.push({ path, body });
    return write(path, body);
  });
  return {
    fetchMock,
    posts,
    reads: () => fetchMock.mock.calls.map(([input]) => String(input)),
    uninstall,
  };
}
