import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { App } from '../App';
import { defaultApiResponse, jsonResponse } from './fixtures';

const HEALTH = '/api/v1/health';

const fetchMock = vi.fn<typeof fetch>();

/** 组合根当前一条真实证据源都没接上，后端如实回"全部未知"。 */
const allUnknown = {
  overall: 'unknown',
  evidence_complete: false,
  sections: [
    {
      code: 'recovery-point',
      title: '恢复点年龄',
      state: 'unknown',
      detail: '恢复点证据不可读，无法判断灾难恢复能力',
    },
    {
      code: 'owner-liveness',
      title: '各 owner 活性',
      state: 'unknown',
      detail: '没有任何 owner 活性证据',
    },
  ],
};

beforeEach(() => {
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) =>
    String(input).startsWith(HEALTH)
      ? jsonResponse(allUnknown)
      : defaultApiResponse(input),
  );
  vi.stubGlobal('fetch', fetchMock);
});

/** 本机健康从「待我动作」视图挪进了顶栏状态浮层：等工作台读到后点开胶囊。 */
async function openStatus() {
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('table');
  await user.click(screen.getByRole('button', { name: /^工作台状态/ }));
  return user;
}

it('状态浮层挂上了健康页并走既有内容 API 的同一个前缀', async () => {
  await openStatus();

  await screen.findByRole('region', { name: '本机健康' });
  const requested = fetchMock.mock.calls.map((call) => String(call[0]));
  expect(requested).toContain(HEALTH);
  expect(requested.every((path) => path.startsWith('/api/'))).toBe(true);
});

it('后端如实回全部未知时健康页显示未知且明说证据不完整', async () => {
  await openStatus();

  const panel = await screen.findByRole('region', { name: '本机健康' });
  await waitFor(() =>
    expect(
      within(panel).getByRole('list', { name: '健康检查项' }),
    ).toBeInTheDocument(),
  );
  expect(within(panel).getByText('整体状态').nextSibling).toHaveAttribute(
    'data-state',
    'unknown',
  );
  expect(within(panel).getByText(/证据不完整/)).toBeInTheDocument();
  expect(within(panel).queryByText('正常')).not.toBeInTheDocument();
});

it('恢复点尚未建立时显示既定中文并且整体不显示绿', async () => {
  fetchMock.mockImplementation(async (input) =>
    String(input).startsWith(HEALTH)
      ? jsonResponse({
          overall: 'unknown',
          evidence_complete: false,
          sections: [
            {
              code: 'recovery-point',
              title: '恢复点年龄',
              state: 'unknown',
              detail: '尚未建立灾难恢复点',
            },
          ],
        })
      : defaultApiResponse(input),
  );

  await openStatus();

  const panel = await screen.findByRole('region', { name: '本机健康' });
  await waitFor(() =>
    expect(within(panel).getByText('尚未建立灾难恢复点')).toBeInTheDocument(),
  );
  expect(within(panel).queryByText('正常')).not.toBeInTheDocument();
});

it('健康页请求失败时只影响健康页本身其余视图照常', async () => {
  fetchMock.mockImplementation(async (input) =>
    String(input).startsWith(HEALTH)
      ? jsonResponse({ error: '本机未装配健康证据来源。' }, 404)
      : defaultApiResponse(input),
  );

  await openStatus();

  const panel = await screen.findByRole('region', { name: '本机健康' });
  await waitFor(() =>
    expect(within(panel).getByText(/未知不等于正常/)).toBeInTheDocument(),
  );
  expect(within(panel).queryByText('正常')).not.toBeInTheDocument();
  expect(
    screen.getByRole('heading', { level: 1, name: '驾驶舱' }),
  ).toBeInTheDocument();
});

it('离开状态浮层（去别的视图）后不再保留健康页', async () => {
  const user = await openStatus();
  await screen.findByRole('region', { name: '本机健康' });

  await user.click(screen.getByRole('button', { name: '项目' }));

  expect(screen.queryByRole('region', { name: '本机健康' })).toBeNull();
});
