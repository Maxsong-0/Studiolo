import { screen, within } from '@testing-library/react';
import type { UserEvent } from '@testing-library/user-event';
import { vi, type Mock } from 'vitest';
import { installPolicyWriter } from '../../policyApi';
import { ApiError } from '../../sessionHttp';
import { jsonResponse } from '../fixtures';

/** 策略平面界面测试的共用夹具：读走 fetch，写走 installPolicyWriter（与 policyPanel.test 同一做法）。 */

export const HASH = 'b'.repeat(64);
export const DESIRED_ID = '01ABCDEFGHJKMNPQRSTVWXYZ00';

type Body = Record<string, unknown>;

/** 策略页的三个子标签（T-198）。 */
export type PolicyTabName = '现状' | '待处理' | '提交';

/** 子标签按钮；「待处理」的名字后面带件数，所以按开头找。 */
export function tab(name: PolicyTabName) {
  return screen.getByRole('tab', { name: new RegExp(`^${name}`) });
}

/** 点一个子标签。 */
export async function openTab(user: UserEvent, name: PolicyTabName) {
  await user.click(tab(name));
}

/** 「待处理」右边那一格：选中草案的细节，发布、校验、撤回的结果也落在这里。 */
export function detailCell() {
  return screen.getByRole('region', { name: '选中的草案' });
}

/** 一份完整的能力集（与后端 CapabilitySet 同形）；测试在它的副本上改。 */
const CAPABILITY = {
  ai_type: 'claude',
  read: {
    project_whitelist: ['项目/Studiolo', '项目/旧项目'],
    deny_by_default: true,
    rules_whitelist: ['CLAUDE.md'],
  },
  write: {
    board: false,
    board_projects: [] as string[],
    content_patch_only: true,
    content_direct: false,
  },
  policy: { propose_desired: true, activate: false },
  hard_bound_skills: [] as { when: string; must_use: string }[],
};
export type CapabilityShape = typeof CAPABILITY;

/** 能力集正文（JSON 文本，挤在一行——服务端存的就是规范化后的这种）；`change` 在副本上改。 */
export function capabilityJson(
  change: (draft: CapabilityShape) => void = () => {},
): string {
  const draft = structuredClone(CAPABILITY);
  change(draft);
  return JSON.stringify(draft);
}

/**
 * 与 {@link reportBody} 缺省那份报告对得上的草案正文：多读「项目/Semiment」（报告判扩大）、
 * 不再读「项目/旧项目」（报告判收窄）。
 */
export const DRAFT_CAPABILITY = capabilityJson((draft) => {
  draft.read.project_whitelist = ['项目/Studiolo', '项目/Semiment'];
});

/** 「提交」里表单的几个输入框。 */
export function submitFields() {
  const form = screen.getByRole('form', { name: '提交规则草案' });
  return {
    form,
    kind: within(form).getByRole('combobox', { name: '类别' }),
    subject: within(form).getByRole('textbox', { name: '对象' }),
    body: within(form).getByRole('textbox', {
      name: '正文（规则为 Markdown，其余为 JSON）',
    }),
    retire: within(form).getByRole('checkbox', { name: '撤销该对象' }),
    submit: within(form).getByRole('button', {
      name: '提交并校验（不发布）',
    }),
  };
}

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

/**
 * 草案列表应答（与接口同形）：每份未发布草案都带两张表，「同一对象里比它更新的未发布草案号」与
 * 「其中挡住它发布的」（服务端只算同一提交者的）。缺省都是空表，即每份都是各自对象最新的那份。
 * 要测先后就用 `newer` 指明（草案号 → 更新的草案号），要测挡住发布再用 `blocked` 照服务端的判定写明。
 */
export function listBody(
  desired: Body[],
  newer: Record<string, string[]> = {},
  blocked: Record<string, string[]> = {},
): Body {
  const open = desired.filter((row) =>
    ['pending', 'validated', 'rejected'].includes(String(row.status)),
  );
  const table = (source: Record<string, string[]>) =>
    Object.fromEntries(
      open.map((row) => [String(row.id), source[String(row.id)] ?? []]),
    );
  return { desired, newer_open: table(newer), blocked_by: table(blocked) };
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
  /** 未发布草案的先后：草案号 → 同一对象里比它更新的草案号；缺省每份各自最新。 */
  newer?: Record<string, string[]>;
  /** 挡住发布的：草案号 → 同一提交者更新的草案号（照服务端判定写明）；缺省都不挡。 */
  blocked?: Record<string, string[]>;
  versions?: Body[];
  /** 各版的正文（版本号 → 正文，null＝撤销版本）；没给的那一版正文是「v<版本号> 正文」。 */
  bodies?: Record<number, string | null>;
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
    if (path === '/api/v1/policy/desired')
      return jsonResponse(listBody(desired, options.newer, options.blocked));
    if (path === '/api/v1/policy/versions') return jsonResponse({ versions });
    const match = /^\/api\/v1\/policy\/versions\/(\d+)$/.exec(path);
    const meta = match
      ? versions.find((row) => String(row.version) === match[1])
      : undefined;
    if (meta) {
      const version = Number(meta.version);
      const body =
        options.bodies && version in options.bodies
          ? options.bodies[version]
          : `v${version} 正文`;
      return jsonResponse({ version: { meta, body } });
    }
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
