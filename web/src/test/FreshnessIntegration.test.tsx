import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { App } from '../App';
import { fixture, jsonResponse } from './fixtures';

const fetchMock = vi.fn<typeof fetch>();
const fresh = {
  stale: false,
  indexed_through: '12',
  committed_through: '12',
  watcher_healthy: true,
  unresolved: false,
  reason: null,
};

beforeEach(() => {
  fetchMock.mockReset();
  fetchMock.mockResolvedValue(jsonResponse({ ...fixture, freshness: fresh }));
  vi.stubGlobal('fetch', fetchMock);
});

/** 新鲜度不再挂成每页顶部的横幅，原文收进顶栏状态浮层：先点开胶囊。 */
async function openStatus(user = userEvent.setup()) {
  await user.click(screen.getByRole('button', { name: /^工作台状态/ }));
  return user;
}

/** 工作台请求的先后。状态胶囊在外壳里另读本机健康，那条请求不在这里数。 */
function workbenchRequests() {
  return fetchMock.mock.calls
    .map(([input]) => String(input))
    .filter((path) => path.startsWith('/api/workbench'));
}

it('显示真实原因与未知水位，不把陈旧结果解释为允许编辑', async () => {
  fetchMock.mockResolvedValue(
    jsonResponse({
      ...fixture,
      freshness: {
        ...fresh,
        stale: true,
        committed_through: null,
        unresolved: true,
        reason: 'unresolved',
      },
    }),
  );
  render(<App />);
  await screen.findByRole('table');
  // 不再在每页顶部挂新鲜度横幅：胶囊收起时页面里没有这一块，胶囊上报的是原因。
  expect(
    screen.queryByRole('region', { name: '内容新鲜度' }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole('button', { name: /^工作台状态/ }),
  ).toHaveAccessibleName(/写入结果未决/);
  await openStatus();
  const notice = await screen.findByRole('region', { name: '内容新鲜度' });
  expect(notice).toHaveTextContent('快照已陈旧');
  expect(notice).toHaveTextContent('存在结果未决的写入');
  expect(notice).toHaveTextContent('提交水位 未知');
  expect(notice).toHaveTextContent('文档编辑还需独立授权');
  expect(notice).not.toHaveTextContent('已追平已知提交');
});

it.each([undefined, null])('服务缺少证据时明确未知 %s', async (freshness) => {
  fetchMock.mockResolvedValue(jsonResponse({ ...fixture, freshness }));
  render(<App />);
  await openStatus();
  expect(await screen.findByText('新鲜度未知')).toBeInTheDocument();
});

it.each(['success', 'failure', 'malformed'] as const)(
  '重新核验到 %s 保持焦点，等待期间不沿用旧成功状态或重复发请求',
  async (outcome) => {
    render(<App />);
    const user = await openStatus();
    const control = await screen.findByRole('button', { name: '重新核验' });
    let finish!: (value: Response) => void;
    fetchMock.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    control.focus();
    await user.keyboard('{Enter}');
    expect(control).toHaveFocus();
    expect(control).toHaveAttribute('aria-disabled', 'true');
    expect(control).not.toBeDisabled();
    expect(screen.getByText('正在重新核验')).toBeInTheDocument();
    expect(screen.queryByText('已追平已知提交')).not.toBeInTheDocument();
    await user.keyboard('{Enter}');
    expect(workbenchRequests()).toEqual([
      '/api/workbench',
      '/api/workbench?refresh=true',
    ]);
    await act(async () =>
      finish(
        outcome === 'failure'
          ? jsonResponse({ error: '暂不可用' }, 503)
          : jsonResponse({
              ...fixture,
              freshness: outcome === 'malformed' ? [] : fresh,
            }),
      ),
    );
    expect(control).toHaveFocus();
    expect(control).toHaveAttribute('aria-disabled', 'false');
    const notice = screen.getByRole('region', { name: '内容新鲜度' });
    expect(notice).toHaveTextContent(
      outcome === 'success' ? '已追平已知提交' : '本次核验失败',
    );
    if (outcome !== 'success') {
      expect(
        within(notice).queryByText('已追平已知提交'),
      ).not.toBeInTheDocument();
    }
  },
);

it('未知但规范的服务端原因按纯文本解释，不被当作新鲜', async () => {
  fetchMock.mockResolvedValue(
    jsonResponse({
      ...fixture,
      freshness: {
        ...fresh,
        stale: true,
        reason: 'future_check',
      },
    }),
  );
  render(<App />);
  await openStatus();
  expect(
    await screen.findByText('服务端报告了新的核验原因：future_check。'),
  ).toBeInTheDocument();
});
