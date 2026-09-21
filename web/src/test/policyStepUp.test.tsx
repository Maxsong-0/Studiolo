import { StrictMode } from 'react';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { PolicyBoundary } from '../PolicyBoundary';
import { SessionBoundary } from '../SessionBoundary';
import { ownerScope } from './canonicalFixtures';
import { jsonResponse } from './fixtures';
import { csrf, pair, sessionResponse } from './sessionFixtures';

const HASH = 'b'.repeat(64);
const MOVED = 'c'.repeat(64);
const DESIRED_ID = '01ABCDEFGHJKMNPQRSTVWXYZ00';
const CODE = '123456';

function status(effective: string, l3 = true) {
  return {
    effective_hash: effective,
    entries: [
      {
        kind: 'capability_set',
        subject: 'claude',
        version: 1,
        body_sha256: HASH,
        activated_at: '2026-09-20T00:00:00Z',
      },
    ],
    global_revoked: false,
    pending_desired: 1,
    revocations: [],
    l3_available: l3,
  };
}

const record = {
  id: DESIRED_ID,
  kind: 'capability_set',
  subject: 'claude',
  body: '{}',
  body_sha256: HASH,
  submitter_kind: 'owner',
  agent_type: null,
  agent_instance: null,
  submitted_at: '2026-09-20T00:00:00Z',
  status: 'validated',
  report: {
    ok: true,
    kind: 'capability_set',
    subject: 'claude',
    retire: false,
    problems: [],
    diff: {
      format: 'json',
      entries: [],
      added: 0,
      removed: 0,
      changed: 0,
      truncated: false,
    },
    expansions: [],
    narrowings: [],
    base_effective_hash: HASH,
    current_version: 1,
    body_sha256: HASH,
  },
  validated_at: '2026-09-20T00:00:00Z',
  activated_version: null,
};

function activated(effective: string) {
  return {
    activation: {
      version: 2,
      kind: 'capability_set',
      subject: 'claude',
      retired: false,
      body_sha256: HASH,
      effective_hash: effective,
      previous_effective_hash: HASH,
      expansions: [],
      desired_id: DESIRED_ID,
      rollback_of: null,
      cleared_type_revocation: false,
      activated_by: 'owner',
    },
    audit_confirmed: true,
  };
}

// 每次现造一个响应：同一个 Response 反复 clone() 会在流被 tee 多层后抛 TypeError，
// 表现为「发布未完成，请核对当前 effective。」而不是 403——那是夹具坏了，不是被挡下。
function denied() {
  return jsonResponse(
    {
      code: 'reauthentication_required',
      error: '发布策略需要先完成二次认证（One-time Code）。',
    },
    403,
  );
}

const fetchMock = vi.fn<typeof fetch>();
const activations: unknown[] = [];
const stepUps: unknown[] = [];
let effective = HASH;
let l3Available = true;
let factors: unknown = null;
let activateReply: (attempt: number) => Response;
let stepUpReply: () => Response;

function now() {
  const verified = Date.now();
  return { verified_at_ms: verified, expires_at_ms: verified + 299_000 };
}

beforeEach(() => {
  activations.length = 0;
  stepUps.length = 0;
  effective = HASH;
  l3Available = true;
  factors = {
    observed_at_ms: Date.now(),
    recent_authentication: null,
    recovery_candidate_available: false,
    pending: false,
  };
  activateReply = () => jsonResponse(activated(MOVED));
  stepUpReply = () => jsonResponse(now());
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input, init) => {
    const path = String(input);
    const body =
      init?.body === undefined ? undefined : JSON.parse(String(init.body));
    if (path === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (path === '/api/v1/auth/state') return jsonResponse({ factors });
    if (path === '/api/v1/auth/step-up') {
      stepUps.push(body);
      return stepUpReply();
    }
    if (path === '/api/v1/policy')
      return jsonResponse(status(effective, l3Available));
    if (path === '/api/v1/policy/desired')
      return jsonResponse({ desired: [record] });
    if (path === '/api/v1/policy/versions')
      return jsonResponse({ versions: [] });
    if (path === '/api/v1/policy/activate') {
      activations.push(body);
      return activateReply(activations.length - 1);
    }
    return sessionResponse(input, init);
  });
  vi.stubGlobal('fetch', fetchMock);
});

async function openPolicy() {
  const user = userEvent.setup();
  render(
    <StrictMode>
      <SessionBoundary>
        {(account) => (
          <>
            {account}
            <PolicyBoundary />
          </>
        )}
      </SessionBoundary>
    </StrictMode>,
  );
  await screen.findByRole('table', { name: '现行各项' });
  return user;
}

/**
 * 选中待处理的那份 desired，点发布，等二次认证窗口出现。
 *
 * 每步都自证：按钮可点、发布请求确实发出，失败时一眼看出卡在哪一步，
 * 不必从整页 DOM 里猜（[[分支断言必须自证走了哪一支]]）。
 */
async function publish(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole('button', { name: '能力集 claude' }));
  const button = await screen.findByRole('button', {
    name: '发布（需二次认证）',
  });
  expect(button).toBeEnabled();
  await user.click(button);
  await waitFor(() => expect(activations).toHaveLength(1));
  return screen.findByRole('dialog', { name: '二次认证后继续' });
}

/** 已发出的策略状态读取次数：被 L3 挡下时不该增加。 */
function statusReads() {
  return fetchMock.mock.calls.filter(
    ([path]) => String(path) === '/api/v1/policy',
  ).length;
}

it('发布被二次认证挡下时就地弹窗，认证通过后按原 hash 自动重试同一次发布', async () => {
  const user = await openPolicy();
  activateReply = (attempt) =>
    attempt === 0 ? denied() : jsonResponse(activated(MOVED));
  const loaded = statusReads();
  const dialog = await publish(user);

  // 被挡下＝服务端什么也没改：此时再刷新列表只会让面板一直 busy，把人认证完的重试吞掉。
  expect(statusReads()).toBe(loaded);

  expect(
    within(dialog).getByText(/本次动作：发布 能力集 claude/),
  ).toBeInTheDocument();
  expect(
    screen.queryByText(/在「账户」里完成二次认证/),
  ).not.toBeInTheDocument();
  expect(activations).toHaveLength(1);

  await user.type(within(dialog).getByLabelText('验证码'), CODE);
  await user.click(
    within(dialog).getByRole('button', { name: '完成认证并继续' }),
  );

  await screen.findByText(/已发布为 v2/);
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  expect(statusReads()).toBeGreaterThan(loaded);
  expect(stepUps).toEqual([{ code: CODE }]);
  // 重试与首次逐字节相同：人审过哪一份 effective 就发布哪一份，验证码不混进策略请求。
  expect(activations).toHaveLength(2);
  expect(activations[1]).toEqual(activations[0]);
  expect(activations[1]).toEqual({
    desired_id: DESIRED_ID,
    expected_effective_hash: HASH,
  });
  expect(JSON.stringify(activations)).not.toContain(CODE);
  const stepUp = fetchMock.mock.calls.find(
    ([path]) => path === '/api/v1/auth/step-up',
  );
  expect(stepUp?.[1]).toMatchObject({
    method: 'POST',
    credentials: 'same-origin',
    headers: { 'X-CSRF-Token': csrf },
  });
});

it('等认证期间 effective 变了，重试仍用人审过的那份 hash，被服务端挡下而不是改用新的', async () => {
  const user = await openPolicy();
  activateReply = (attempt) =>
    attempt === 0
      ? denied()
      : jsonResponse(
          { code: 'hash_mismatch', error: 'effective 已变化。' },
          409,
        );
  const dialog = await publish(user);
  effective = MOVED;

  await user.type(within(dialog).getByLabelText('验证码'), CODE);
  await user.click(
    within(dialog).getByRole('button', { name: '完成认证并继续' }),
  );

  expect(await screen.findByRole('alert')).toHaveTextContent(
    'effective 已变化',
  );
  expect(activations).toHaveLength(2);
  expect(activations[1]).toEqual({
    desired_id: DESIRED_ID,
    expected_effective_hash: HASH,
  });
});

it('取消认证就不发布：既不提交验证码，也不重试原动作', async () => {
  const user = await openPolicy();
  activateReply = () => denied();
  const dialog = await publish(user);

  await user.click(
    within(dialog).getByRole('button', { name: '取消，不发布' }),
  );

  await waitFor(() =>
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument(),
  );
  expect(stepUps).toHaveLength(0);
  expect(activations).toHaveLength(1);
});

it('验证码不通过时留在窗口里等新码，绝不替人重试发布', async () => {
  const user = await openPolicy();
  activateReply = () => denied();
  stepUpReply = () => jsonResponse({ error: '验证码无效。' }, 403);
  const dialog = await publish(user);

  await user.type(within(dialog).getByLabelText('验证码'), CODE);
  await user.click(
    within(dialog).getByRole('button', { name: '完成认证并继续' }),
  );

  expect(await within(dialog).findByRole('alert')).toHaveTextContent(
    '验证或请求未通过',
  );
  expect(within(dialog).getByLabelText('验证码')).toHaveValue('');
  expect(stepUps).toHaveLength(1);
  expect(activations).toHaveLength(1);
});

it('本入口没有 One-time Code 路径时，窗口直说不能发布，不给假的认证入口', async () => {
  factors = null;
  const user = await openPolicy();
  activateReply = () => denied();
  const dialog = await publish(user);

  expect(await within(dialog).findByRole('alert')).toHaveTextContent(
    '本入口未启用二次认证',
  );
  expect(within(dialog).queryByLabelText('验证码')).not.toBeInTheDocument();
  expect(stepUps).toHaveLength(0);
  expect(activations).toHaveLength(1);
});
