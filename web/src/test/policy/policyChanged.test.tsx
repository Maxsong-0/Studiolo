import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../../App';
import { PolicyBoundary } from '../../PolicyBoundary';
import { PolicyChangedContext } from '../../policy/policyChanged';
import { ApiError } from '../../sessionHttp';
import { defaultApiResponse } from '../fixtures';
import {
  desiredBody,
  detailCell,
  openTab,
  policyServer,
  submitFields,
  tab,
  versionBody,
  type PolicyServer,
} from './policyFixtures';

/**
 * 「策略平面刚被写过」的通知（T-198）：策略页、规则页每做完一件写动作发一次，App 那一份草案读取
 * （顶栏「驾驶舱 N」角标用的）据此安静重读，角标不必等人回到驾驶舱。只读的、被挡下没发生的不发。
 */

const FIRST = '01ABCDEFGHJKMNPQRSTVWXYZ01';
const SECOND = '01ABCDEFGHJKMNPQRSTVWXYZ02';

let server: PolicyServer | undefined;
afterEach(() => {
  server?.uninstall();
  server = undefined;
});

/** 单独挂策略页，外面包一个记次数的通知。 */
function mount() {
  const changed = vi.fn<() => void>();
  render(
    <PolicyChangedContext.Provider value={changed}>
      <PolicyBoundary />
    </PolicyChangedContext.Provider>,
  );
  return changed;
}

/** 整份快照读了几次（第一项是 /api/v1/policy）。 */
const snapshots = () =>
  server?.reads().filter((path) => path === '/api/v1/policy').length ?? 0;

describe('什么时候算「刚被写过」', () => {
  it('一件写动作做成了（重新校验）：通知一次', async () => {
    server = policyServer({
      desired: [desiredBody()],
      write: () => ({ desired: desiredBody() }),
    });
    window.location.hash = '#view=policy&panel=pending';
    const user = userEvent.setup();
    const changed = mount();
    await screen.findByRole('list', { name: '待处理草案' });
    expect(changed).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: '重新校验' }));
    expect(
      await within(detailCell()).findByText('已重新校验：已校验，可发布。'),
    ).toBeInTheDocument();
    expect(changed).toHaveBeenCalledTimes(1);
  });

  it('没做成也通知一次：可能已经写了一半，同样要重读', async () => {
    server = policyServer({
      desired: [desiredBody()],
      write: () => {
        throw new ApiError('撤回失败。', 500);
      },
    });
    window.location.hash = '#view=policy&panel=pending';
    const user = userEvent.setup();
    const changed = mount();
    await screen.findByRole('list', { name: '待处理草案' });

    await user.click(screen.getByRole('button', { name: '撤回草案' }));
    expect(await within(detailCell()).findByText('撤回失败。')).toHaveAttribute(
      'role',
      'alert',
    );
    expect(changed).toHaveBeenCalledTimes(1);
  });

  it('发布被二次认证挡下：动作没有发生，不通知', async () => {
    // 缺省的写接口：发布一律回 403「需要二次认证」。
    server = policyServer({ desired: [desiredBody()] });
    window.location.hash = '#view=policy&panel=pending';
    const user = userEvent.setup();
    const changed = mount();
    await screen.findByRole('list', { name: '待处理草案' });

    await user.click(
      screen.getByRole('button', { name: '发布（需二次认证）' }),
    );
    await user.click(screen.getByRole('button', { name: '确认发布' }));
    await screen.findByRole('dialog', { name: '二次认证后继续' });
    expect(server.posts.map((post) => post.path)).toEqual([
      '/api/v1/policy/activate',
    ]);
    expect(changed).not.toHaveBeenCalled();
  });

  it('读一版正文不算写：不通知，也不重读整份数据', async () => {
    server = policyServer({ versions: [versionBody(2), versionBody(1)] });
    const user = userEvent.setup();
    const changed = mount();
    await screen.findByRole('list', { name: '版本列表' });
    expect(snapshots()).toBe(1);

    await user.click(screen.getByRole('button', { name: 'v2' }));
    expect(await screen.findByText('v2 正文')).toBeInTheDocument();
    expect(await screen.findByText('已读取 v2。')).toHaveAttribute(
      'role',
      'status',
    );
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'v1' })).toBeEnabled(),
    );
    expect(changed).not.toHaveBeenCalled();
    expect(snapshots()).toBe(1);
  });

  it('表单的本地校验没过、没发请求：不通知', async () => {
    server = policyServer();
    const user = userEvent.setup();
    const changed = mount();
    await screen.findByRole('table', { name: '生效中的各项' });
    await openTab(user, '提交');
    await user.click(submitFields().submit);
    expect(within(submitFields().form).getByRole('alert')).toHaveTextContent(
      '请填写对象',
    );
    expect(server.posts).toEqual([]);
    expect(changed).not.toHaveBeenCalled();
  });
});

describe('顶栏「驾驶舱 N」角标跟着策略页的写入更新', () => {
  /** 顶栏「驾驶舱」旁的计数。 */
  function badge() {
    return screen
      .getByRole('button', { name: '驾驶舱' })
      .querySelector('.shell-nav-count')?.textContent;
  }

  it('在策略页撤回一份草案：角标跟着少一件，人不必回驾驶舱', async () => {
    const listed = [
      desiredBody({ id: FIRST }),
      desiredBody({ id: SECOND, subject: 'codex' }),
    ];
    server = policyServer({
      desired: listed,
      write: (path) => {
        if (path !== '/api/v1/policy/withdraw') throw new ApiError('未知', 404);
        const [gone] = listed.splice(0, 1);
        return { desired: { ...gone, status: 'withdrawn' } };
      },
    });
    // 策略平面走上面那一份；其余接口（工作台快照等）走全站的默认应答。
    const policy = server.fetchMock;
    vi.stubGlobal(
      'fetch',
      vi.fn<typeof fetch>(async (input, init) =>
        String(input).startsWith('/api/v1/policy')
          ? policy(input, init)
          : defaultApiResponse(input),
      ),
    );
    window.location.hash = '#view=policy&panel=pending';
    const user = userEvent.setup();
    render(<App />);

    // 工作台待办 1 件，加策略里等你的两份草案。
    await waitFor(() => expect(badge()).toBe('3'));
    await screen.findByRole('list', { name: '待处理草案' });
    expect(tab('待处理')).toHaveTextContent('待处理 2');

    await user.click(screen.getByRole('button', { name: '撤回草案' }));
    expect(
      await within(detailCell()).findByText('已撤回这份草案。'),
    ).toBeInTheDocument();
    expect(server.posts).toEqual([
      { path: '/api/v1/policy/withdraw', body: { desired_id: FIRST } },
    ]);
    await waitFor(() => expect(badge()).toBe('2'));
    // 人一直在策略页的「待处理」。
    expect(tab('待处理')).toHaveAttribute('aria-selected', 'true');
    expect(tab('待处理')).toHaveTextContent('待处理 1');
  });
});
