import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { installFileOperationWriter } from '../fileOperationApi';
import { renaming, renameResponse } from './renameFixtures';
import { jsonResponse } from './fixtures';
import { deferred } from './sessionFixtures';
import type { FileOperationRequest } from '../fileOperationModel';
import { setup, service, start, fresh, accept } from './renameBrowserFixtures';

it('关闭原dialog后迟到的fresh结果不能重新打开目标', async () => {
  const waiting = deferred<Response>();
  const fetcher = service(() => waiting.promise);
  const uninstall = installFileOperationWriter(accept);
  const view = setup();
  try {
    await start();
    await screen.findByRole('button', { name: '正在核验目标…' });
    fireEvent.click(screen.getByRole('button', { name: '关闭编辑器' }));
    expect(screen.getByLabelText('已打开路径')).toHaveTextContent('closed');
    await act(async () => {
      waiting.resolve(jsonResponse(fresh));
    });
    expect(screen.getByLabelText('已打开路径')).toHaveTextContent('closed');
    expect(
      fetcher.mock.calls.some(([input]) =>
        String(input).includes(encodeURIComponent(renaming.to)),
      ),
    ).toBe(false);
    expect(view.files.getSnapshot()?.phase).toBe('committed');
  } finally {
    view.unmount();
    uninstall();
  }
});
it('关闭时POST仍在途，迟到Committed留在同Store但不触发目标导航', async () => {
  const fetcher = service(async () => jsonResponse(fresh));
  const pending = deferred<unknown>();
  const uninstall = installFileOperationWriter(() => pending.promise);
  const view = setup();
  try {
    await start();
    fireEvent.click(screen.getByRole('button', { name: '关闭编辑器' }));
    await act(async () => {
      pending.resolve(renameResponse());
    });
    expect(screen.getByLabelText('已打开路径')).toHaveTextContent('closed');
    expect(view.files.getSnapshot()?.phase).toBe('committed');
    expect(
      fetcher.mock.calls.some(
        ([input]) => String(input) === '/api/workbench?refresh=true',
      ),
    ).toBe(false);
  } finally {
    view.unmount();
    uninstall();
  }
});
it('未知原键核对取得完整回执后，仍在原dialog内才自动核验目标', async () => {
  service(async () => jsonResponse(fresh));
  let attempts = 0;
  const writer = vi.fn(async (request: FileOperationRequest) => {
    if (++attempts === 1) throw new TypeError('lost outcome');
    return accept(request);
  });
  const uninstall = installFileOperationWriter(writer);
  const view = setup();
  try {
    await start();
    await screen.findByText(/重命名结果尚未确认/);
    expect(screen.getByLabelText('已打开路径')).toHaveTextContent(
      renaming.from,
    );
    fireEvent.click(screen.getByRole('button', { name: '核对原请求' }));
    await waitFor(() =>
      expect(screen.getByLabelText('已打开路径')).toHaveTextContent(
        renaming.to,
      ),
    );
    expect(writer).toHaveBeenCalledTimes(2);
    expect(writer.mock.calls[0][0]).toEqual(writer.mock.calls[1][0]);
  } finally {
    view.unmount();
    uninstall();
  }
});
