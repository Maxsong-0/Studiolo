import { act, renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { PolicyChangedContext } from '../../policy/policyChanged';
import { publishPreview, withdrawPreview } from '../../policy/previews';
import {
  usePolicyActions,
  type Activation,
  type PolicyActionOptions,
} from '../../policy/usePolicyActions';
import { installPolicyWriter } from '../../policyApi';
import { desiredEnvelope, policyStatus } from '../../policyModel';
import { ApiError } from '../../sessionHttp';
import { DESIRED_ID, HASH, desiredBody, statusBody } from './policyFixtures';

/**
 * 共用写动作钩子为驾驶舱的就地审批（T-200）加的几个入口：撤回先出执行预览、校验结果的说法由调用方给、
 * 不重复通知全站那一份草案读取、关掉认证窗口后提示照实改写。原有行为由策略页、规则页的用例守着。
 */

const RECORD = desiredEnvelope({ desired: desiredBody() });
const ORIGIN = `review:${DESIRED_ID}`;
const WITHDRAW_KEY = `withdraw:${DESIRED_ID}`;
const DONE = '已撤回 能力集 claude（草案 01ABCDEFGHJK）。';

const ACTIVATION: Activation = {
  request: { desired_id: DESIRED_ID, expected_effective_hash: HASH },
  label: '能力集 claude',
  action: '发布 能力集 claude',
  origin: ORIGIN,
  supersedes: 0,
  remains: 0,
};

let uninstall: (() => void) | undefined;
afterEach(() => {
  uninstall?.();
  uninstall = undefined;
});

/** 挂上钩子：写走假的受控会话写者，外面包一个记次数的「策略平面刚被写过」。 */
function mount(
  write: (path: string, body: unknown) => unknown,
  options?: PolicyActionOptions,
) {
  const posts: { path: string; body: unknown }[] = [];
  uninstall = installPolicyWriter(async (path, body) => {
    posts.push({ path, body });
    return write(path, body);
  });
  const reload = vi.fn<() => Promise<void>>(async () => {});
  const changed = vi.fn<() => void>();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <PolicyChangedContext.Provider value={changed}>
      {children}
    </PolicyChangedContext.Provider>
  );
  const view = renderHook(() => usePolicyActions(reload, options), {
    wrapper,
  });
  /** 等这一件做完（重读也读完）。 */
  const idle = () =>
    waitFor(() => expect(view.result.current.busy).toBe(false));
  return { result: view.result, posts, reload, changed, idle };
}

const withdrawn = () => ({ desired: desiredBody({ status: 'withdrawn' }) });
const validated = () => ({ desired: desiredBody({ status: 'validated' }) });
const denied = () => {
  throw new ApiError('发布策略需要先完成二次认证。', 403, {
    code: 'reauthentication_required',
  });
};

describe('撤回先出执行预览（驾驶舱的「退回」）', () => {
  const open = (
    result: ReturnType<typeof mount>['result'],
    key = WITHDRAW_KEY,
  ) =>
    act(() =>
      result.current.previewWithdraw(
        key,
        ORIGIN,
        RECORD,
        withdrawPreview(RECORD),
        DONE,
      ),
    );

  it('只打开确认框，不发请求；同一个按钮再点一次收起', () => {
    const { result, posts, reload } = mount(withdrawn);
    open(result);
    expect(result.current.preview).toMatchObject({
      key: WITHDRAW_KEY,
      kind: 'withdraw',
      origin: ORIGIN,
      done: DONE,
      content: { title: '撤回 能力集 claude', confirmLabel: '确认撤回' },
    });
    expect(posts).toEqual([]);
    open(result);
    expect(result.current.preview).toBeNull();
    expect(posts).toEqual([]);
    expect(reload).not.toHaveBeenCalled();
  });

  it('换一份草案的按钮：预览换成那一份，不是收起', () => {
    const { result } = mount(withdrawn);
    open(result);
    open(result, 'withdraw:other');
    expect(result.current.preview?.key).toBe('withdraw:other');
  });

  it('确认才撤回：发出撤回请求，说的是调用方给的那一句，落在发起的那一块，然后重读', async () => {
    const { result, posts, reload, idle } = mount(withdrawn);
    open(result);
    await act(async () => {
      result.current.confirmPreview();
    });
    await idle();
    expect(posts).toEqual([
      { path: '/api/v1/policy/withdraw', body: { desired_id: DESIRED_ID } },
    ]);
    expect(result.current.preview).toBeNull();
    expect(result.current.notice).toEqual({ origin: ORIGIN, text: DONE });
    expect(result.current.error).toBeNull();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('取消：收起，不发请求', () => {
    const { result, posts } = mount(withdrawn);
    open(result);
    act(() => result.current.closePreview());
    expect(result.current.preview).toBeNull();
    expect(posts).toEqual([]);
  });

  it('策略页、规则页那种点了就撤的「撤回草案」照旧：不出预览，说的还是原来那一句', async () => {
    const { result, posts, idle } = mount(withdrawn);
    await act(async () => {
      result.current.withdraw('queue', RECORD);
    });
    await idle();
    expect(posts.map((post) => post.path)).toEqual(['/api/v1/policy/withdraw']);
    expect(result.current.notice).toEqual({
      origin: 'queue',
      text: '已撤回这份草案。',
    });
  });
});

describe('校验之后说的那一句', () => {
  it('不给就是原来的说法', async () => {
    const { result, idle } = mount(validated);
    await act(async () => {
      result.current.validate('queue', RECORD);
    });
    await idle();
    expect(result.current.notice).toEqual({
      origin: 'queue',
      text: '已重新校验：已校验，可发布。',
    });
  });

  it('调用方给了就用它的：拿到的是校验之后的那一份草案', async () => {
    const { result, posts, idle } = mount(() => ({
      desired: desiredBody({ status: 'rejected' }),
    }));
    await act(async () => {
      result.current.validate(
        ORIGIN,
        RECORD,
        (updated) => `校验之后是 ${updated.status}`,
      );
    });
    await idle();
    expect(posts).toEqual([
      { path: '/api/v1/policy/validate', body: { desired_id: DESIRED_ID } },
    ]);
    expect(result.current.notice).toEqual({
      origin: ORIGIN,
      text: '校验之后是 rejected',
    });
  });
});

describe('要不要发「策略平面刚被写过」', () => {
  it('缺省发：做成了发一次', async () => {
    const { result, changed, reload, idle } = mount(withdrawn);
    await act(async () => {
      result.current.withdraw('queue', RECORD);
    });
    await idle();
    expect(changed).toHaveBeenCalledTimes(1);
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('没做成也发一次（缺省）', async () => {
    const { result, changed, reload, idle } = mount(() => {
      throw new ApiError('撤回失败。', 500);
    });
    await act(async () => {
      result.current.withdraw('queue', RECORD);
    });
    await idle();
    expect(result.current.error).toEqual({
      origin: 'queue',
      text: '撤回失败。',
    });
    expect(changed).toHaveBeenCalledTimes(1);
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('announce: false（驾驶舱）：做成了不发，照样重读自己那一份', async () => {
    const { result, changed, reload, idle } = mount(withdrawn, {
      announce: false,
    });
    await act(async () => {
      result.current.withdraw(ORIGIN, RECORD);
    });
    await idle();
    expect(result.current.notice?.text).toBe('已撤回这份草案。');
    expect(changed).not.toHaveBeenCalled();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('announce: false：没做成也不发，照样重读', async () => {
    const { result, changed, reload, idle } = mount(
      () => {
        throw new ApiError('撤回失败。', 500);
      },
      { announce: false },
    );
    await act(async () => {
      result.current.withdraw(ORIGIN, RECORD);
    });
    await idle();
    expect(result.current.error).toEqual({
      origin: ORIGIN,
      text: '撤回失败。',
    });
    expect(changed).not.toHaveBeenCalled();
    expect(reload).toHaveBeenCalledTimes(1);
  });
});

describe('被二次认证挡下之后，人关掉了认证窗口', () => {
  async function blocked(options?: PolicyActionOptions) {
    const mounted = mount(denied, options);
    const { result } = mounted;
    act(() =>
      result.current.togglePreview(
        `publish:${DESIRED_ID}`,
        ACTIVATION,
        publishPreview(
          RECORD,
          RECORD.report!,
          policyStatus(statusBody()),
          [],
          [],
        ),
      ),
    );
    await act(async () => {
      result.current.confirmPreview();
    });
    await mounted.idle();
    return mounted;
  }

  it('被挡下：记下这一次，提示请人在窗口里输入验证码；什么都没写，不重读、不通知', async () => {
    const { result, posts, reload, changed } = await blocked();
    expect(posts.map((post) => post.path)).toEqual(['/api/v1/policy/activate']);
    expect(result.current.pending).toMatchObject({ ...ACTIVATION, attempt: 0 });
    expect(result.current.notice).toEqual({
      origin: ORIGIN,
      text: '发布 能力集 claude 需要二次认证：请在弹出的窗口输入一次性验证码，通过后会自动继续这一次动作。',
    });
    expect(reload).not.toHaveBeenCalled();
    expect(changed).not.toHaveBeenCalled();
  });

  it('关掉窗口：不再等着重试，提示照实改成「已取消，没有执行」，不再写着请在窗口里输入', async () => {
    const { result, posts, reload, changed } = await blocked();
    act(() => result.current.closeStepUp());
    expect(result.current.pending).toBeNull();
    expect(result.current.notice).toEqual({
      origin: ORIGIN,
      text: '已取消二次认证：发布 能力集 claude 没有执行。',
    });
    // 没有替人重试，也没有别的请求。
    expect(posts).toHaveLength(1);
    expect(reload).not.toHaveBeenCalled();
    expect(changed).not.toHaveBeenCalled();
  });

  it('没有等着认证的动作时关窗口：什么都不改', () => {
    const { result } = mount(withdrawn);
    act(() => result.current.closeStepUp());
    expect(result.current.pending).toBeNull();
    expect(result.current.notice).toBeNull();
  });
});
