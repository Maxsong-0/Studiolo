import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { PolicyBoundary } from '../PolicyBoundary';
import { installPolicyWriter } from '../policyApi';
import { ApiError } from '../sessionHttp';
import { jsonResponse } from './fixtures';

const HASH = 'b'.repeat(64);
const STATUS = '/api/v1/policy';

function status(overrides: Record<string, unknown> = {}) {
  return {
    effective_hash: HASH,
    entries: [
      {
        kind: 'capability_set',
        subject: 'claude',
        version: 1,
        body_sha256: HASH,
        activated_at: '2026-09-17T00:00:00Z',
      },
    ],
    global_revoked: false,
    pending_desired: 0,
    revocations: [],
    l3_available: true,
    ...overrides,
  };
}

function reportFor(ok: boolean) {
  return {
    ok,
    kind: 'capability_set',
    subject: 'claude',
    retire: false,
    problems: ok ? [] : [{ path: 'policy.activate', message: '永久为 false' }],
    diff: {
      format: 'json',
      entries: [],
      added: 0,
      removed: 0,
      changed: 0,
      truncated: false,
    },
    expansions: ok
      ? [
          {
            path: 'read.project_whitelist',
            before: null,
            after: '项目/Semiment',
            note: '读取范围变宽',
          },
        ]
      : [],
    narrowings: [],
    base_effective_hash: HASH,
    current_version: 1,
    body_sha256: HASH,
  };
}

function desired(ok: boolean) {
  return {
    desired: {
      id: '01ABCDEFGHJKMNPQRSTVWXYZ00',
      kind: 'capability_set',
      subject: 'claude',
      body: '{}',
      body_sha256: HASH,
      submitter_kind: 'owner',
      agent_type: null,
      agent_instance: null,
      submitted_at: '2026-09-17T00:00:00Z',
      status: ok ? 'validated' : 'rejected',
      report: reportFor(ok),
      validated_at: '2026-09-17T00:00:00Z',
      activated_version: null,
    },
  };
}

const fetchMock = vi.fn<typeof fetch>();
const posts: { path: string; body: unknown }[] = [];
let uninstall = () => {};

beforeEach(() => {
  posts.length = 0;
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) => {
    const path = String(input);
    if (path === STATUS) return jsonResponse(status());
    if (path === '/api/v1/policy/desired') return jsonResponse({ desired: [] });
    if (path === '/api/v1/policy/versions')
      return jsonResponse({ versions: [] });
    return jsonResponse({ error: '未知' }, 404);
  });
  vi.stubGlobal('fetch', fetchMock);
  uninstall = installPolicyWriter(async (path, body) => {
    posts.push({ path, body });
    if (path === '/api/v1/policy/desired') return desired(true);
    if (path === '/api/v1/policy/activate')
      throw new ApiError('发布策略需要先完成二次认证。', 403, {
        code: 'reauthentication_required',
      });
    if (path === '/api/v1/agents/revoke')
      return { created: true, level: 'type', target: 'claude' };
    throw new ApiError('未知', 404);
  });
});
afterEach(() => {
  uninstall();
  vi.unstubAllGlobals();
});

it('提交 desired 后把扩大项标红，发布被挡在二次认证后且永不自动生效', async () => {
  const user = userEvent.setup();
  render(<PolicyBoundary />);
  await screen.findByRole('table', { name: '现行各项' });
  expect(screen.getAllByText(HASH.slice(0, 12)).length).toBeGreaterThan(0);

  const form = screen.getByRole('form', { name: '提交 desired' });
  await user.clear(within(form).getByRole('textbox', { name: 'subject' }));
  await user.type(
    within(form).getByRole('textbox', { name: 'subject' }),
    'claude',
  );
  await user.click(
    within(form).getByRole('button', { name: '提交并校验（不发布）' }),
  );

  await screen.findByRole('alert');
  expect(screen.getByText(/1 项权限扩大/)).toBeInTheDocument();
  const expansions = screen.getByRole('list', { name: '权限扩大项' });
  expect(within(expansions).getByText('读取范围变宽')).toBeInTheDocument();
  expect(posts.map((post) => post.path)).toEqual(['/api/v1/policy/desired']);
  expect(posts[0].body).toMatchObject({
    kind: 'capability_set',
    subject: 'claude',
  });

  await user.click(screen.getByRole('button', { name: '发布（需二次认证）' }));
  await waitFor(() =>
    expect(
      screen.getByText(/需要先在「账户」里完成二次认证/),
    ).toBeInTheDocument(),
  );
  expect(posts.at(-1)).toMatchObject({
    path: '/api/v1/policy/activate',
    body: {
      desired_id: '01ABCDEFGHJKMNPQRSTVWXYZ00',
      expected_effective_hash: HASH,
    },
  });
});

it('吊销即刻提交，未接入策略平面时显示未接入而不是空白', async () => {
  const user = userEvent.setup();
  render(<PolicyBoundary />);
  const form = await screen.findByRole('form', { name: '吊销' });
  await user.selectOptions(
    within(form).getByRole('combobox', { name: '级别' }),
    'type',
  );
  await user.type(
    within(form).getByRole('textbox', { name: '目标' }),
    'claude',
  );
  await user.type(within(form).getByRole('textbox', { name: '理由' }), '测试');
  await user.click(
    within(form).getByRole('button', { name: '吊销（即刻生效）' }),
  );
  await screen.findByText(/已吊销 类型 claude/);
  expect(posts.at(-1)).toMatchObject({
    path: '/api/v1/agents/revoke',
    body: { level: 'type', target: 'claude', reason: '测试' },
  });

  fetchMock.mockImplementation(async () =>
    jsonResponse({ code: 'policy_plane_absent', error: '未装配' }, 404),
  );
  render(<PolicyBoundary />);
  await screen.findByText(/本入口未装配策略平面/);
});

// 让同批请求先全部进入，再结束鉴权；稳定模拟服务端的两个并发许可。
function authLimitedReads() {
  const respond = fetchMock.getMockImplementation()!;
  let active = 0;
  let denied = 0;
  fetchMock.mockImplementation(async (input, init) => {
    const overloaded = ++active > 2;
    try {
      await Promise.resolve();
      await Promise.resolve();
      if (overloaded) {
        denied += 1;
        return jsonResponse({ error: '认证并发预算已满' }, 503);
      }
      return await respond(input, init);
    } finally {
      active -= 1;
    }
  });
  return () => denied;
}

it('首次加载在两个认证并发许可下读完整策略，不自触发503', async () => {
  const denied = authLimitedReads();
  render(<PolicyBoundary />);
  await screen.findByRole('table', { name: '现行各项' });
  expect(
    screen.getByRole('form', { name: '提交 desired' }),
  ).toBeInTheDocument();
  expect(denied()).toBe(0);
  expect(posts).toHaveLength(0);
});

it('提交后的刷新也遵守认证预算，读完后保留成功提示且不重放提交', async () => {
  const user = userEvent.setup();
  render(<PolicyBoundary />);
  const form = await screen.findByRole('form', { name: '提交 desired' });
  const denied = authLimitedReads();
  await user.type(
    within(form).getByRole('textbox', { name: 'subject' }),
    'claude',
  );
  const submit = within(form).getByRole('button', {
    name: '提交并校验（不发布）',
  });
  await user.click(submit);
  await waitFor(() => expect(submit).toBeEnabled());
  expect(screen.getByRole('status')).toHaveTextContent('已提交并校验通过');
  expect(screen.queryByText('认证并发预算已满')).not.toBeInTheDocument();
  expect(denied()).toBe(0);
  expect(posts.map((post) => post.path)).toEqual(['/api/v1/policy/desired']);
});

it('首项读取失败就显示错误，不继续读取其余列表或自动重试', async () => {
  fetchMock.mockImplementation(async () =>
    jsonResponse({ error: '读取不可用' }, 503),
  );
  render(<PolicyBoundary />);
  expect(await screen.findByRole('alert')).toHaveTextContent('读取不可用');
  expect(fetchMock.mock.calls.map(([path]) => path)).toEqual([STATUS]);
  expect(posts).toHaveLength(0);
});
