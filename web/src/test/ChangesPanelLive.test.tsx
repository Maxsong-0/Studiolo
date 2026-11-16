import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import type { Revalidate } from '../live/autoRefresh';

// 面板自己的调度器换成手动的：测试决定什么时候「到点」，不等真的 20 秒。
const live = vi.hoisted(() => ({
  readers: [] as Revalidate[],
  options: [] as { active: boolean }[],
}));
vi.mock('../live/useAutoRefresh', async (original) => {
  const actual = await original<typeof import('../live/useAutoRefresh')>();
  const { NO_AUTO_REFRESH } = await import('../live/autoRefresh');
  const scheduler = {
    ...NO_AUTO_REFRESH,
    register: (reader: Revalidate) => {
      live.readers.push(reader);
      return () => {
        const index = live.readers.indexOf(reader);
        if (index >= 0) live.readers.splice(index, 1);
      };
    },
  };
  return {
    ...actual,
    useAutoRefresh: (options: { active: boolean }) => {
      live.options.push(options);
      return scheduler;
    },
  };
});

import { Documents } from '../components/Documents';
import { installSessionReader } from '../sessionHttp';
import { changesFixture } from '../../dev/fixtures/changes';
import { fixture } from './fixtures';

it('变更面板到点只问水位：变了亮「有新变化」，点一下才重读列表', async () => {
  history.replaceState(null, '', '#view=documents&panel=changes');
  const page = changesFixture(
    Date.now(),
    21,
    null,
    '40000000-0000-4000-8000-000000000001',
    false,
  );
  let sequence = page.through.sequence;
  const isHead = (url: string) => url.startsWith('/api/v1/changes/head');
  const reader = vi.fn(async (url: string) =>
    isHead(url)
      ? { vault_uuid: page.vault_uuid, through: { ...page.through, sequence } }
      : url.startsWith('/api/v1/changes')
        ? page
        : { generation: 7, hits: [], truncated: false },
  );
  const lists = () =>
    reader.mock.calls.filter(
      ([url]) => url.startsWith('/api/v1/changes') && !isHead(url),
    ).length;
  const uninstall = installSessionReader(reader);
  try {
    const user = userEvent.setup();
    render(
      <Documents
        projects={fixture.projects}
        generation={7}
        onOpen={vi.fn()}
        onRefresh={vi.fn()}
      />,
    );
    const panel = screen.getByRole('tabpanel', { name: '变更' });
    await within(panel).findByRole('heading', { name: '项目/Studiolo' });
    expect(live.options.at(-1)?.active).toBe(true);
    const tick = () =>
      act(async () => {
        for (const read of [...live.readers])
          await read(new AbortController().signal);
      });
    await tick();
    expect(within(panel).queryByText(/有新变化/)).not.toBeInTheDocument();
    expect(lists()).toBe(1);

    sequence = '22';
    await tick();
    expect(within(panel).getByText(/有新变化/)).toBeInTheDocument();
    expect(lists()).toBe(1);
    // 提示亮着就不用再问：这一路停下，等人点。
    expect(live.options.at(-1)?.active).toBe(false);

    await user.click(within(panel).getByRole('button', { name: '读取新变化' }));
    await waitFor(() =>
      expect(within(panel).queryByText(/有新变化/)).not.toBeInTheDocument(),
    );
    expect(lists()).toBe(2);
  } finally {
    uninstall();
  }
});
