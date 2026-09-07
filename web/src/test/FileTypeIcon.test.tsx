import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { FileTypeIcon } from '../components/FileTypeIcon';
import { FileTree } from '../components/FileTree';
import { jsonResponse } from './fixtures';
import { treeEntry, treePage } from './treeFixtures';

it('图标仅按末尾后缀分类，支持中文路径与大写后缀，目录和Markdown优先', () => {
  const cases = [
    [treeEntry('资料.png/笔记.MD', 'markdown'), 'book'],
    [treeEntry('资料/照片.png', 'directory'), 'folder'],
    [treeEntry('目录/山河.JPEG'), 'image'],
    [treeEntry('目录/流程.SVG'), 'image'],
    [treeEntry('目录/统计.CSV'), 'table'],
    [treeEntry('目录/预算.XLS'), 'table'],
    [treeEntry('目录/预算.xlsx'), 'table'],
    [treeEntry('目录/构想.canvas'), 'canvas'],
    [treeEntry('目录/草稿.excalidraw'), 'canvas'],
    [treeEntry('目录/展示.HTML'), 'code'],
    [treeEntry('目录/配置.json'), 'code'],
    [treeEntry('目录/脚本.py'), 'code'],
    [treeEntry('照片.png/没有后缀'), 'attachment'],
    [treeEntry('目录/照片.png.backup'), 'attachment'],
    [treeEntry('目录/报告.pdf'), 'attachment'],
    [treeEntry('目录/末尾.'), 'attachment'],
  ] as const;
  const { container, rerender } = render(<FileTypeIcon entry={cases[0][0]} />);
  for (const [entry, expected] of cases) {
    rerender(<FileTypeIcon entry={entry} />);
    const icon = container.querySelector('.file-type-icon');
    expect(icon).toHaveClass(`file-type-${expected}`);
    expect(icon).toHaveAttribute('aria-hidden', 'true');
    expect(icon?.querySelector('svg')).toHaveAttribute('aria-hidden', 'true');
  }
});

it('文件树与附件弹窗复用图标，不增加可访问名称或附件内容请求', async () => {
  const attachment = treeEntry('中文图示.SVG');
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockResolvedValue(jsonResponse(treePage('', [attachment])));
  vi.stubGlobal('fetch', fetchMock);
  const user = userEvent.setup();
  render(<FileTree generation={7} onOpen={vi.fn()} onRefresh={vi.fn()} />);
  const trigger = await screen.findByRole('button', {
    name: '中文图示.SVG，查看附件信息',
  });
  expect(trigger.querySelector('.file-type-image')).toBeInTheDocument();
  await user.click(trigger);
  const dialog = screen.getByRole('dialog', { name: '附件信息' });
  expect(dialog.querySelector('.file-type-image')).toHaveClass(
    'file-type-badge',
  );
  expect(within(dialog).getByText('.SVG')).toBeInTheDocument();
  expect(within(dialog).getByText('9007199254740993 字节')).toBeInTheDocument();
  expect(within(dialog).queryByRole('img')).not.toBeInTheDocument();
  expect(dialog.querySelector('img, iframe, object, embed')).toBeNull();
  expect(fetchMock).toHaveBeenCalledTimes(1);
});
