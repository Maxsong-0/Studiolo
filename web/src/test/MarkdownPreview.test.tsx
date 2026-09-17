import { render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { MarkdownPreview } from '../components/MarkdownPreview';
import { EditorTextBoundary } from '../components/EditorTextBoundary';
import { changedLines } from '../textComparison';

it('GFM、中文与frontmatter安全呈现，HTML/图片/脚本/mermaid没有执行入口', () => {
  const content =
    '---\n未知: 保留\n---\n# 标题\n\n- [x] 完成\n\n| 列 | 值 |\n|---|---|\n| 中 | 文 |\n\n<script>alert(1)</script>\n\n![图](https://example.invalid/a.svg)\n\n[恶意](javascript:alert(1))\n\n```mermaid\ngraph TD; A-->B\n```';
  const { container } = render(<MarkdownPreview content={content} />);
  expect(screen.getByRole('heading', { name: '标题' })).toBeInTheDocument();
  expect(screen.getByRole('table')).toHaveTextContent('中文');
  expect(screen.getByText('Frontmatter 原文')).toBeInTheDocument();
  expect(
    container.querySelector('script, iframe, img, svg, object, style'),
  ).toBeNull();
  expect(container.querySelector('a[href^="javascript:"]')).toBeNull();
  expect(screen.getByText('<script>alert(1)</script>')).toBeInTheDocument();
  expect(screen.getByText('graph TD; A-->B')).toBeInTheDocument();
});
it('外部安全链接只在用户点击时打开，复杂结构退回全文文本', () => {
  const { rerender } = render(
    <MarkdownPreview content="[文档](https://example.invalid/docs)" />,
  );
  expect(screen.getByRole('link', { name: '文档' })).toHaveAttribute(
    'rel',
    'noreferrer noopener',
  );
  const content = 'a **b** '.repeat(5000);
  rerender(<MarkdownPreview content={content} />);
  expect(screen.getByRole('alert')).toHaveTextContent('退回安全纯文本');
  expect(screen.getByText(content.trim())).toBeInTheDocument();
});
it('渲染意外失败保留草稿全文', () => {
  const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
  function Failure(): never {
    throw new Error('虚构渲染失败');
  }
  render(
    <EditorTextBoundary content="必须保留的草稿">
      <Failure />
    </EditorTextBoundary>,
  );
  expect(screen.getByRole('alert')).toHaveTextContent('保留安全纯文本');
  expect(screen.getByText('必须保留的草稿')).toBeInTheDocument();
  errors.mockRestore();
});
it('线性差异标记保留共同首尾，不把新增行后的所有正文都标错', () => {
  expect(changedLines('头\n旧\n尾', '头\n新\n新增\n尾')).toEqual([
    { text: '头', changed: false },
    { text: '新', changed: true },
    { text: '新增', changed: true },
    { text: '尾', changed: false },
  ]);
});
