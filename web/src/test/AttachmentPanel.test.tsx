import { Blob as NodeBlob } from 'node:buffer';
import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { AttachmentPanel } from '../components/AttachmentPanel';
import { treeEntry } from './treeFixtures';
import { deferred } from './sessionFixtures';

const fetchMock = vi.fn<typeof fetch>();
const created = vi.fn<(blob: Blob) => string>(() => 'blob:attachment-preview');
const revoked = vi.fn();
beforeEach(() => {
  fetchMock.mockReset();
  created.mockClear();
  revoked.mockClear();
  vi.stubGlobal('fetch', fetchMock);
  vi.stubGlobal('Blob', NodeBlob);
  vi.stubGlobal(
    'URL',
    class extends URL {
      static createObjectURL = created;
      static revokeObjectURL = revoked;
    },
  );
});
afterEach(() => vi.unstubAllGlobals());

function reply(
  bytes: string,
  mime: string,
  length = new TextEncoder().encode(bytes).length,
) {
  return new Response(bytes, {
    headers: { 'Content-Type': mime, 'Content-Length': String(length) },
  });
}
function show(path: string, size: number) {
  const entry = treeEntry(path, 'attachment', String(size));
  const close = vi.fn();
  return {
    ...render(<AttachmentPanel entry={entry} generation={7} onClose={close} />),
    entry,
    close,
  };
}

it('HTML 不预取，主动准备后只有惰性下载，未知响应 MIME 同样不会执行', async () => {
  const body = '<script>alert(1)</script>';
  fetchMock.mockResolvedValue(reply(body, 'text/html'));
  const { container } = show('危险.html', body.length);
  expect(fetchMock).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole('button', { name: '准备下载' }));
  const link = await screen.findByRole('link', { name: '下载附件' });
  expect(link).toHaveAttribute('download', '危险.html');
  expect(link).toHaveFocus();
  expect(link).not.toHaveAttribute('target');
  expect(container.querySelector('img,iframe,object,script')).toBeNull();
  expect(created.mock.calls[0]).toBeDefined();
  expect(created.mock.calls[0][0].type).toBe('application/octet-stream');
});

it('图片自动读并支持原尺寸，卸载后撤销 URL，读取携带认证头', async () => {
  fetchMock.mockResolvedValue(reply('image', 'image/png'));
  const { unmount } = show('图片.png', 5);
  expect(await screen.findByRole('img', { name: '图片.png' })).toHaveAttribute(
    'src',
    'blob:attachment-preview',
  );
  expect(fetchMock.mock.calls[0][1]).toMatchObject({
    credentials: 'same-origin',
    cache: 'no-store',
    redirect: 'error',
    headers: { 'X-Studiolo-Request': '1' },
  });
  await userEvent.click(screen.getByRole('button', { name: '原始尺寸' }));
  expect(screen.getByRole('button', { name: '适应宽度' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  unmount();
  expect(revoked).toHaveBeenCalledExactlyOnceWith('blob:attachment-preview');
});

it('CSV 中脚本只作文本节点，内容大于文本上限时不自动取', async () => {
  const text = '<script>alert(1)</script>';
  fetchMock.mockResolvedValue(reply(text, 'text/plain; charset=utf-8'));
  const { unmount, container } = show('表格.csv', text.length);
  expect(await screen.findByText(text)).toHaveProperty('tagName', 'PRE');
  expect(container.querySelector('script')).toBeNull();
  unmount();
  fetchMock.mockClear();
  show('很大.csv', 1024 * 1024 + 1);
  expect(screen.getByRole('button', { name: '准备下载' })).toBeInTheDocument();
  expect(fetchMock).not.toHaveBeenCalled();
});

it('长度不符不生成 URL，超限只提示到同步副本打开', async () => {
  fetchMock.mockResolvedValue(reply('bad', 'image/png', 8));
  const { unmount } = show('图片.png', 3);
  expect(await screen.findByRole('alert')).toHaveTextContent(
    '附件大小与当前快照不符',
  );
  expect(created).not.toHaveBeenCalled();
  unmount();
  fetchMock.mockClear();
  show('超限.zip', 32 * 1024 * 1024 + 1);
  expect(
    screen.queryByRole('button', { name: '准备下载' }),
  ).not.toBeInTheDocument();
  expect(fetchMock).not.toHaveBeenCalled();
});

it('关闭中止在途读取，忽略取消的迟到响应不会建 URL', async () => {
  const pending = deferred<Response>();
  fetchMock.mockReturnValue(pending.promise);
  const { close } = show('图片.png', 3);
  await userEvent.click(screen.getByRole('button', { name: '关闭附件信息' }));
  expect(fetchMock.mock.calls[0][1]?.signal?.aborted).toBe(true);
  await act(async () => pending.resolve(reply('png', 'image/png')));
  expect(created).not.toHaveBeenCalled();
  expect(close).toHaveBeenCalledOnce();
});

it('换代次中止旧读取，旧成功结果不能落进新代次', async () => {
  const pending = deferred<Response>();
  fetchMock
    .mockReturnValueOnce(pending.promise)
    .mockResolvedValue(reply('new', 'image/png'));
  const { entry, rerender } = show('图片.png', 3);
  const oldSignal = fetchMock.mock.calls[0][1]?.signal;
  rerender(
    <AttachmentPanel
      key="8"
      entry={entry}
      generation={8}
      onClose={() => undefined}
    />,
  );
  expect(oldSignal?.aborted).toBe(true);
  await screen.findByRole('img');
  await act(async () => pending.resolve(reply('old', 'image/png')));
  expect(created).toHaveBeenCalledOnce();
});
