import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DocumentDialog } from '../components/DocumentDialog';
import { emptyNavigation, jsonResponse } from './fixtures';

const fetchMock = vi.fn<typeof fetch>();
const onClose = vi.fn();
const onRefresh = vi.fn();

beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal('fetch', fetchMock);
});

describe('原文异步读取', () => {
  it('较早请求的迟到响应不会覆盖新来源', async () => {
    let finishOld!: (response: Response) => void;
    fetchMock.mockImplementationOnce(
      () =>
        new Promise<Response>((resolve) => {
          finishOld = resolve;
        }),
    );
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        path: 'new.md',
        title: '新原文',
        content: '最新内容',
        navigation: emptyNavigation,
      }),
    );
    const { rerender } = render(
      <DocumentDialog
        source={{ path: 'old.md', line: 1 }}
        generation={1}
        onClose={onClose}
        onRefresh={onRefresh}
      />,
    );
    rerender(
      <DocumentDialog
        source={{ path: 'new.md', line: 1 }}
        generation={1}
        onClose={onClose}
        onRefresh={onRefresh}
      />,
    );
    expect(await screen.findByText('最新内容')).toBeInTheDocument();
    expect(fetchMock.mock.calls[0][1]?.signal?.aborted).toBe(true);
    await act(async () =>
      finishOld(
        jsonResponse({
          path: 'old.md',
          title: '旧原文',
          content: '过时内容',
          navigation: emptyNavigation,
        }),
      ),
    );
    expect(screen.queryByText('过时内容')).not.toBeInTheDocument();
    expect(screen.getByText('最新内容')).toBeInTheDocument();
  });

  it('普通原文错误可重试当前来源，无需伪造内容', async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ error: '暂时无法读取' }, 500),
    );
    fetchMock.mockResolvedValueOnce(
      jsonResponse({
        path: 'source.md',
        title: '恢复原文',
        content: '已恢复',
        navigation: emptyNavigation,
      }),
    );
    const user = userEvent.setup();
    render(
      <DocumentDialog
        source={{ path: 'source.md', line: 1 }}
        generation={9}
        onClose={onClose}
        onRefresh={onRefresh}
      />,
    );
    expect(await screen.findByRole('alert')).toHaveTextContent('暂时无法读取');
    await user.click(screen.getByRole('button', { name: '重试' }));
    expect(await screen.findByText('已恢复')).toBeInTheDocument();
    expect(fetchMock.mock.calls[0][0]).toBe(fetchMock.mock.calls[1][0]);
  });
});
