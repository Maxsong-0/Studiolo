import { useFileOperationStore } from '../fileOperationContext';
import { useState } from 'react';
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { RecentlyDeleted } from '../components/RecentlyDeleted';
import {
  FileOperationBoundary,
  FileOperationScopeBoundary,
} from '../FileOperationBoundary';
import { installFileOperationWriter } from '../fileOperationApi';
import type { FileOperationRequest } from '../fileOperationModel';
import { canonical, ownerScope } from './canonicalFixtures';
import { deleting } from './fileOperationFixtures';
import { deferred } from './sessionFixtures';
import { jsonResponse } from './fixtures';

// 外层真实 Store 留在页面，只卸载弹窗和它的 Status，不用测试 setter 注入未知态。
function Page({ onChanged }: { onChanged: () => void }) {
  const { store, record } = useFileOperationStore();
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        onClick={() => {
          void store.submit(deleting, new AbortController().signal);
        }}
      >
        发起原请求
      </button>
      <button onClick={() => setOpen(true)}>打开最近删除</button>
      <output data-testid="operation-phase">{record?.phase ?? 'empty'}</output>
      <output data-testid="original-request">
        {JSON.stringify(record?.request ?? null)}
      </output>
      {open ? (
        <RecentlyDeleted
          vault={canonical.vault_uuid}
          onClose={() => setOpen(false)}
          onChanged={onChanged}
        />
      ) : null}
    </>
  );
}

it('原请求重试延迟时关闭弹窗，迟到失败不刷新或重读列表且保留原键', async () => {
  const late = deferred<unknown>();
  const writer = vi
    .fn<
      (request: FileOperationRequest, signal: AbortSignal) => Promise<unknown>
    >()
    .mockRejectedValueOnce(new Error('首次响应丢失'))
    .mockImplementationOnce(() => late.promise);
  const uninstall = installFileOperationWriter(writer);
  const fetcher = vi.fn(async (input: RequestInfo | URL) => {
    if (!String(input).startsWith('/api/v1/tombstones?')) {
      throw new Error(`未预期的列表请求：${String(input)}`);
    }
    return jsonResponse({ entries: [], next_cursor: null });
  });
  vi.stubGlobal('fetch', fetcher);
  const changed = vi.fn();
  const view = render(
    <FileOperationBoundary>
      <FileOperationScopeBoundary scope={ownerScope}>
        <Page onChanged={changed} />
      </FileOperationScopeBoundary>
    </FileOperationBoundary>,
  );
  try {
    fireEvent.click(screen.getByRole('button', { name: '发起原请求' }));
    await waitFor(() =>
      expect(screen.getByTestId('operation-phase')).toHaveTextContent(
        'uncertain',
      ),
    );
    const original = screen.getByTestId('original-request').textContent;
    expect(JSON.parse(original!)).toEqual(deleting);
    fireEvent.click(screen.getByRole('button', { name: '打开最近删除' }));
    await screen.findByText('这一页没有可恢复的删除记录。');
    expect(fetcher).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole('button', { name: '核对原请求' }));
    expect(writer).toHaveBeenCalledTimes(2);
    expect(writer.mock.calls[1][0]).toEqual(writer.mock.calls[0][0]);
    const signal = writer.mock.calls[1][1];
    expect(signal.aborted).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: '关闭' }));
    await waitFor(() =>
      expect(
        screen.queryByRole('dialog', { name: '最近删除' }),
      ).not.toBeInTheDocument(),
    );
    expect(signal.aborted).toBe(true);
    expect(screen.getByTestId('operation-phase')).toHaveTextContent('sending');

    // 模拟写者在取消之后才返回；Store 仍收口原 Promise，旧 UI 不能发出新读取。
    await act(async () => {
      late.reject(new Error('取消后迟到的网络失败'));
      await late.promise.catch(() => {});
    });
    await waitFor(() =>
      expect(screen.getByTestId('operation-phase')).toHaveTextContent(
        'uncertain',
      ),
    );
    expect(changed).not.toHaveBeenCalled();
    expect(fetcher).toHaveBeenCalledOnce();
    expect(screen.getByTestId('original-request').textContent).toBe(original);
    const leave = new Event('beforeunload', { cancelable: true });
    window.dispatchEvent(leave);
    expect(leave.defaultPrevented).toBe(true);

    // 重新打开属于新弹窗寿命，允许真实重读一次，仍显示同一原请求的恢复入口。
    fireEvent.click(screen.getByRole('button', { name: '打开最近删除' }));
    await screen.findByText('这一页没有可恢复的删除记录。');
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(screen.getByRole('button', { name: '核对原请求' })).toBeEnabled();
    expect(screen.getByTestId('original-request').textContent).toBe(original);
    expect(writer).toHaveBeenCalledTimes(2);
  } finally {
    view.unmount();
    uninstall();
  }
});
