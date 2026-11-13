import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { Documents } from '../components/Documents';
import { installSessionReader } from '../sessionHttp';
import { installChangesWriter } from '../changesApi';
import { changesFixture } from '../../dev/fixtures/changes';
import { fixture } from './fixtures';
it('变更标签可由 URL 直达，按项目分组，显式已读后重取并保留原文入口', async () => {
  history.replaceState(null, '', '#view=documents&panel=changes');
  let marked = false;
  const snapshot = '40000000-0000-4000-8000-000000000001';
  const reader = vi.fn(async (url: string) =>
    url.startsWith('/api/v1/changes')
      ? changesFixture(Date.now(), 21, null, snapshot, marked)
      : { generation: 7, hits: [], truncated: false },
  );
  const unread = installSessionReader(reader);
  const writer = vi.fn(async () => {
    marked = true;
    const p = changesFixture(Date.now(), 21, null, snapshot, true);
    return {
      status: 'seen',
      vault_uuid: p.vault_uuid,
      project: p.project,
      through: p.through,
    };
  });
  const unwrite = installChangesWriter(writer);
  try {
    const open = vi.fn();
    const user = userEvent.setup();
    render(
      <Documents
        projects={fixture.projects}
        generation={7}
        onOpen={open}
        onRefresh={vi.fn()}
      />,
    );
    const panel = screen.getByRole('tabpanel', { name: '变更' });
    await within(panel).findByRole('heading', { name: '项目/Studiolo' });
    expect(within(panel).getByText(/外部变化 · 作者未知/)).toBeInTheDocument();
    expect(within(panel).getByText(/服务已重新启动/)).toBeInTheDocument();
    expect(writer).not.toHaveBeenCalled();
    await user.click(
      within(panel).getAllByRole('button', { name: '打开当前文档' })[0],
    );
    expect(open).toHaveBeenCalledWith({
      path: '项目/Studiolo/00-总览/INDEX.md',
      line: 1,
    });
    await user.click(
      within(panel).getByRole('button', { name: '将本次快照全部标为已读' }),
    );
    await within(panel).findByText('此快照内的变化已标记为已读。');
    expect(within(panel).queryByLabelText('未读')).not.toBeInTheDocument();
  } finally {
    unwrite();
    unread();
  }
});
