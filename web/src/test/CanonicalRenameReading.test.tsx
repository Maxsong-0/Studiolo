import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { expect, it } from 'vitest';
import { installFileOperationWriter } from '../fileOperationApi';
import { renaming } from './renameFixtures';
import { jsonResponse } from './fixtures';
import { setup, service, start, fresh, accept } from './renameBrowserFixtures';
import { deferred } from './sessionFixtures';

it('新目标保留可导出的原回执；明确收起后才允许下一文件操作', async () => {
  service(async () => jsonResponse(fresh), undefined, true);
  const uninstall = installFileOperationWriter(accept);
  const view = setup();
  try {
    await start();
    await waitFor(() =>
      expect(screen.getByLabelText('已打开路径')).toHaveTextContent(
        renaming.to,
      ),
    );
    await screen.findByDisplayValue('独立读取的当前目标');
    const receipt = await screen.findByRole('region', { name: '文件操作回执' });
    expect(receipt).toHaveTextContent(renaming.from);
    expect(receipt).toHaveTextContent(renaming.to);
    expect(screen.getByRole('button', { name: '导出请求信息' })).toBeEnabled();
    expect(
      screen.queryByRole('button', { name: '重新读取目标' }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: '重命名文件' })).toBeDisabled();
    expect(view.files.getSnapshot()?.request.kind).toBe('rename');
    fireEvent.click(screen.getByRole('button', { name: '收起回执' }));
    expect(view.files.getSnapshot()).toBeNull();
    await waitFor(() =>
      expect(screen.getByRole('button', { name: '重命名文件' })).toBeEnabled(),
    );
    expect(screen.getByRole('button', { name: '删除文件' })).toBeEnabled();
  } finally {
    view.unmount();
    uninstall();
  }
});

it('Committed后先读取真实新鲜快照，再创建目标Controller；目标新HTTP授权仍可只读', async () => {
  const fetcher = service(async () => jsonResponse(fresh));
  const uninstall = installFileOperationWriter(accept);
  const view = setup();
  try {
    await start();
    await waitFor(() =>
      expect(screen.getByLabelText('已打开路径')).toHaveTextContent(
        renaming.to,
      ),
    );
    expect(
      await screen.findByDisplayValue('独立读取的当前目标'),
    ).toHaveAttribute('readonly');
    const urls = fetcher.mock.calls.map(([input]) => String(input));
    const freshIndex = urls.findIndex(
      (url) => url === '/api/workbench?refresh=true',
    );
    const targetIndex = urls.findIndex(
      (url) =>
        url.startsWith('/api/v1/file?') &&
        new URL(url, 'https://fixture.invalid').searchParams.get('path') ===
          renaming.to,
    );
    expect(freshIndex).toBeGreaterThan(-1);
    expect(targetIndex).toBeGreaterThan(freshIndex);
    expect(view.files.getSnapshot()?.receipt?.effects).toHaveLength(2);
  } finally {
    view.unmount();
    uninstall();
  }
});

it('目标已产生新草稿时收起旧回执不重载正文或恢复文件操作权限', async () => {
  service(async () => jsonResponse(fresh), undefined, true);
  const uninstall = installFileOperationWriter(accept);
  const view = setup();
  try {
    await start();
    const editor = await screen.findByDisplayValue('独立读取的当前目标');
    fireEvent.change(editor, { target: { value: '目标的新草稿' } });
    fireEvent.click(screen.getByRole('button', { name: '收起回执' }));
    expect(view.files.getSnapshot()).toBeNull();
    expect(editor).toHaveValue('目标的新草稿');
    expect(screen.getByRole('button', { name: '重命名文件' })).toBeDisabled();
    expect(screen.getByRole('button', { name: '删除文件' })).toBeDisabled();
    expect(screen.getByRole('button', { name: '立即保存' })).toBeEnabled();
  } finally {
    view.unmount();
    uninstall();
  }
});
it('freshness未追平只自动读一次，手动重新读取才开始下一次且按钮保持挂载', async () => {
  let ready = false;
  const waiting = deferred<Response>();
  const fetcher = service(async () =>
    ready
      ? waiting.promise
      : jsonResponse({
          ...fresh,
          freshness: { ...fresh.freshness, stale: true, reason: 'changed' },
        }),
  );
  const uninstall = installFileOperationWriter(accept);
  const view = setup();
  try {
    await start();
    const retry = await screen.findByRole('button', { name: '重新读取目标' });
    expect(screen.getByLabelText('已打开路径')).toHaveTextContent(
      renaming.from,
    );
    expect(
      screen.getByRole('region', { name: '重新读取重命名目标' }),
    ).toHaveTextContent('尚未取得追平');
    const reads = () =>
      fetcher.mock.calls.filter(
        ([input]) => String(input) === '/api/workbench?refresh=true',
      ).length;
    expect(reads()).toBe(1);
    await act(async () => {});
    expect(reads()).toBe(1);
    ready = true;
    retry.focus();
    fireEvent.click(retry);
    expect(screen.getByRole('button', { name: '正在核验目标…' })).toBe(retry);
    expect(retry).toHaveFocus();
    expect(retry).toHaveAttribute('aria-disabled', 'true');
    await act(async () => {
      waiting.resolve(jsonResponse(fresh));
    });
    await waitFor(() =>
      expect(screen.getByLabelText('已打开路径')).toHaveTextContent(
        renaming.to,
      ),
    );
    expect(reads()).toBe(2);
  } finally {
    view.unmount();
    uninstall();
  }
});
it('目标GET返回别Vault时不显示该正文或建立可编辑基线', async () => {
  service(
    async () => jsonResponse(fresh),
    '87654321-1234-1234-1234-123456789abc',
  );
  const uninstall = installFileOperationWriter(accept);
  const view = setup();
  try {
    await start();
    expect(
      await screen.findByText(
        '目标文档属于不同 Vault，不能沿用重命名回执打开。',
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByDisplayValue('独立读取的当前目标'),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole('textbox', { name: 'Markdown 源码' }),
    ).not.toBeInTheDocument();
  } finally {
    view.unmount();
    uninstall();
  }
});
