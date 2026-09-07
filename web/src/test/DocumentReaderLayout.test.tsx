import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { DocumentDialog } from '../components/DocumentDialog';
import { DocumentNavigation } from '../components/DocumentNavigation';
import { emptyNavigation, jsonResponse } from './fixtures';

const fetchMock = vi.fn<typeof fetch>();
const outline = [
  { title: '阅读体验', level: 1, source: { path: '阅读.md', line: 1 } },
  { title: '连续阅读', level: 2, source: { path: '阅读.md', line: 3 } },
];

beforeEach(() => {
  fetchMock.mockReset();
  fetchMock.mockResolvedValue(
    jsonResponse({
      path: '阅读.md',
      title: '阅读体验',
      content: '# 阅读体验\n\n## 连续阅读\n[[下一页]]',
      navigation: { ...emptyNavigation, outline },
    }),
  );
  vi.stubGlobal('fetch', fetchMock);
});

it('大纲默认展开并标记当前位置所属标题，其余导航仍可折叠', async () => {
  const user = userEvent.setup();
  render(
    <DocumentNavigation
      navigation={{ ...emptyNavigation, outline }}
      onNavigate={vi.fn()}
      currentLine={4}
    />,
  );
  const heading = screen.getByText('大纲（2）').closest('details')!;
  expect(heading).toHaveAttribute('open');
  expect(
    screen.getByRole('button', { name: /连续阅读.*第 3 行/ }),
  ).toHaveAttribute('aria-current', 'location');
  expect(
    screen.getByText('本页链接（0）').closest('details'),
  ).not.toHaveAttribute('open');
  await user.click(screen.getByText('大纲（2）'));
  expect(heading).not.toHaveAttribute('open');
  await user.click(screen.getByText('大纲（2）'));
  expect(heading).toHaveAttribute('open');
});

it('窄屏导航开关默认收起，定位来源后收起且保留正文与后退', async () => {
  const user = userEvent.setup();
  const { container } = render(
    <DocumentDialog
      source={{ path: '阅读.md', line: 1 }}
      generation={3}
      onClose={vi.fn()}
      onRefresh={vi.fn()}
    />,
  );
  const toggle = await screen.findByRole('button', { name: '显示导航' });
  const panel = container.querySelector('#document-navigation-panel')!;
  expect(toggle).toHaveAttribute('aria-expanded', 'false');
  expect(toggle).toHaveAttribute('aria-controls', panel.id);
  expect(panel).not.toHaveClass('is-open');
  await user.click(toggle);
  expect(screen.getByRole('button', { name: '隐藏导航' })).toHaveAttribute(
    'aria-expanded',
    'true',
  );
  expect(panel).toHaveClass('is-open');
  await user.click(screen.getByRole('button', { name: /连续阅读.*第 3 行/ }));
  expect(screen.getByRole('button', { name: '显示导航' })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
  expect(panel).not.toHaveClass('is-open');
  expect(
    container.querySelector('.source-lines .target-line'),
  ).toHaveTextContent('3');
  expect(screen.getByLabelText('原文内容')).toHaveTextContent('[[下一页]]');
  expect(
    within(screen.getByLabelText('原文内容')).queryByRole('link'),
  ).toBeNull();
  expect(fetchMock).toHaveBeenCalledTimes(1);
  await user.click(screen.getByRole('button', { name: '后退' }));
  expect(screen.getByText('阅读.md · 第 1 行')).toBeInTheDocument();
});

it('大文档窗口保留真实行号与纯文本，导航不挤入正文滚动容器', async () => {
  const lines = Array.from(
    { length: 400 },
    (_, index) => `${index + 1} ${'字'.repeat(300)}`,
  );
  lines[199] = '<img src=x onerror=alert(1)> [[原始链接]]';
  fetchMock.mockResolvedValueOnce(
    jsonResponse({
      path: '阅读.md',
      title: '大文档',
      content: lines.join('\r\n'),
      navigation: { ...emptyNavigation, outline },
    }),
  );
  const { container } = render(
    <DocumentDialog
      source={{ path: '阅读.md', line: 200 }}
      generation={3}
      onClose={vi.fn()}
      onRefresh={vi.fn()}
    />,
  );
  expect(
    await screen.findByText(/文档较大，仅显示目标附近原文/),
  ).toHaveTextContent('第 100–300 行');
  const source = screen.getByLabelText('原文内容');
  expect(within(source).getAllByRole('listitem')).toHaveLength(201);
  const current = container.querySelector('.source-lines .target-line')!;
  expect(current).toHaveAttribute('aria-current', 'location');
  expect(current.querySelector('.line-number')).toHaveTextContent('200');
  expect(current).toHaveTextContent(
    '<img src=x onerror=alert(1)> [[原始链接]]',
  );
  expect(current.querySelector('img')).toBeNull();
  expect(within(source).queryByRole('navigation')).toBeNull();
});

it('旧快照只提供刷新入口，不回退到新代原文', async () => {
  fetchMock.mockResolvedValueOnce(jsonResponse({ error: '快照已更新' }, 409));
  const user = userEvent.setup();
  const refresh = vi.fn();
  render(
    <DocumentDialog
      source={{ path: '阅读.md', line: 1 }}
      generation={3}
      onClose={vi.fn()}
      onRefresh={refresh}
    />,
  );
  expect(await screen.findByRole('alert')).toHaveTextContent(
    '快照已更新，请刷新工作台',
  );
  expect(screen.queryByRole('button', { name: '重试' })).toBeNull();
  expect(screen.queryByLabelText('原文内容')).toBeNull();
  await user.click(screen.getByRole('button', { name: '刷新工作台' }));
  expect(refresh).toHaveBeenCalledOnce();
  expect(fetchMock).toHaveBeenCalledTimes(1);
});

it('原生 Escape 取消事件调用关闭，卸载后焦点返回触发按钮', async () => {
  const { unmount: removeTrigger } = render(<button>查看来源</button>);
  const trigger = screen.getByRole('button', { name: '查看来源' });
  trigger.focus();
  const close = vi.fn();
  const { unmount } = render(
    <DocumentDialog
      source={{ path: '阅读.md', line: 1 }}
      generation={3}
      onClose={close}
      onRefresh={vi.fn()}
    />,
  );
  await screen.findByRole('heading', { name: '阅读体验' });
  expect(screen.getByRole('button', { name: '关闭原文' })).toHaveAttribute(
    'data-dialog-dismiss',
  );
  fireEvent(
    screen.getByRole('dialog'),
    new Event('cancel', { cancelable: true }),
  );
  await waitFor(() => expect(close).toHaveBeenCalledOnce());
  unmount();
  expect(trigger).toHaveFocus();
  removeTrigger();
});
