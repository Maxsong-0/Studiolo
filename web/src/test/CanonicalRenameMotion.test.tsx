import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { expect, it } from 'vitest';
import { installFileOperationWriter } from '../fileOperationApi';
import { renaming } from './renameFixtures';
import { jsonResponse } from './fixtures';
import { deferred } from './sessionFixtures';
import { mediaPreference } from './dialogSurfaceHelpers';
import { setup, service, start, fresh, accept } from './renameBrowserFixtures';

it('取消关闭动画不复活旧读取，但仍允许新的手动目标核验', async () => {
  mediaPreference(false);
  const waiting = deferred<Response>();
  let reads = 0;
  service(async () => (++reads === 1 ? waiting.promise : jsonResponse(fresh)));
  const uninstall = installFileOperationWriter(accept);
  const view = setup();
  try {
    await start();
    await screen.findByRole('button', { name: '正在核验目标…' });
    const dialog = screen.getByRole('dialog', { name: '编辑与预览' });
    await waitFor(() => expect(dialog.style.opacity).toBe('1'), {
      timeout: 1500,
    });
    fireEvent.click(screen.getByRole('button', { name: '关闭编辑器' }));
    fireEvent.click(screen.getByRole('button', { name: '隐藏预览' }));
    expect(dialog).toHaveAttribute('open');
    await act(async () => {
      waiting.resolve(jsonResponse(fresh));
    });
    const retry = await screen.findByRole('button', { name: '重新读取目标' });
    expect(screen.getByLabelText('已打开路径')).toHaveTextContent(
      renaming.from,
    );
    expect(reads).toBe(1);
    fireEvent.click(retry);
    await waitFor(() =>
      expect(screen.getByLabelText('已打开路径')).toHaveTextContent(
        renaming.to,
      ),
    );
    expect(reads).toBe(2);
  } finally {
    view.unmount();
    uninstall();
  }
});
