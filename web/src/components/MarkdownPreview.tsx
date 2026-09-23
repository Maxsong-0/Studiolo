import { createElement, useMemo } from 'react';
import type { ReactNode } from 'react';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import remarkGfm from 'remark-gfm';
import remarkFrontmatter from 'remark-frontmatter';
import '../styles/document.css';

interface Node {
  type: string;
  children?: Node[];
  value?: string;
  depth?: number;
  ordered?: boolean;
  start?: number | null;
  checked?: boolean | null;
  url?: string;
  alt?: string | null;
}
const parser = unified()
  .use(remarkParse)
  .use(remarkGfm)
  .use(remarkFrontmatter, ['yaml']);
function budget(root: Node) {
  const stack = [{ node: root, depth: 0 }];
  let count = 0;
  while (stack.length) {
    const { node, depth } = stack.pop()!;
    if (++count > 12000 || depth > 48)
      throw new Error('预览结构较复杂，已退回纯文本。');
    for (const child of node.children ?? [])
      stack.push({ node: child, depth: depth + 1 });
  }
}
function safeUrl(url = '') {
  if (/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/u.test(url)) return null;
  try {
    const parsed = new URL(url);
    return ['https:', 'http:', 'mailto:'].includes(parsed.protocol)
      ? parsed.href
      : null;
  } catch {
    return null;
  }
}
function renderNode(node: Node, key: number): ReactNode {
  const children = node.children?.map(renderNode);
  switch (node.type) {
    case 'root':
      return (
        <div key={key} className="markdown-body">
          {children}
        </div>
      );
    case 'text':
      return node.value;
    case 'paragraph':
      return <p key={key}>{children}</p>;
    case 'heading':
      return createElement(
        `h${Math.min(6, Math.max(1, node.depth ?? 1))}`,
        { key },
        children,
      );
    case 'strong':
      return <strong key={key}>{children}</strong>;
    case 'emphasis':
      return <em key={key}>{children}</em>;
    case 'delete':
      return <del key={key}>{children}</del>;
    case 'inlineCode':
      return <code key={key}>{node.value}</code>;
    case 'code':
      return (
        <pre key={key}>
          <code>{node.value}</code>
        </pre>
      );
    case 'html':
      return (
        <pre key={key} className="preview-inert">
          {node.value}
        </pre>
      );
    case 'yaml':
      return (
        <details key={key}>
          <summary>Frontmatter 原文</summary>
          <pre>{node.value}</pre>
        </details>
      );
    case 'blockquote':
      return <blockquote key={key}>{children}</blockquote>;
    case 'list':
      return node.ordered ? (
        <ol key={key} start={node.start ?? 1}>
          {children}
        </ol>
      ) : (
        <ul key={key}>{children}</ul>
      );
    case 'listItem':
      return (
        <li
          key={key}
          className={
            typeof node.checked === 'boolean' ? 'preview-task' : undefined
          }
        >
          {typeof node.checked === 'boolean' ? (
            <span aria-label={node.checked ? '已完成' : '未完成'}>
              {node.checked ? '☑ ' : '☐ '}
            </span>
          ) : null}
          {children}
        </li>
      );
    case 'break':
      return <br key={key} />;
    case 'thematicBreak':
      return <hr key={key} />;
    case 'image':
    case 'imageReference':
      return (
        <span key={key} className="preview-inert">
          [图片：{node.alt || '未命名'}；预览不加载图片]
        </span>
      );
    case 'link': {
      const href = safeUrl(node.url);
      return href ? (
        <a key={key} href={href} target="_blank" rel="noreferrer noopener">
          {children}
        </a>
      ) : (
        <span key={key}>
          {children} <small>（链接未启用）</small>
        </span>
      );
    }
    case 'linkReference':
      return <span key={key}>{children}</span>;
    case 'table':
      return (
        <div className="preview-table" key={key}>
          <table>
            <tbody>{children}</tbody>
          </table>
        </div>
      );
    case 'tableRow':
      return <tr key={key}>{children}</tr>;
    case 'tableCell':
      return <td key={key}>{children}</td>;
    default:
      return <span key={key}>{node.value ?? children}</span>;
  }
}
/**
 * Markdown 预览（编辑器右栏）：标题层级靠字重与上边线区分，代码等宽，表格 1px 线，
 * 链接常驻下划线；正文列不超过 72ch。HTML 按文本显示，不加载图片，不执行图表或脚本。
 */
export function MarkdownPreview({
  content,
  updating = false,
}: {
  content: string;
  updating?: boolean;
}) {
  const result = useMemo(() => {
    try {
      const tree = parser.parse(content) as Node;
      budget(tree);
      return { tree, error: '' };
    } catch {
      return {
        tree: null,
        error: '预览暂不可用或结构超限，已退回安全纯文本。',
      };
    }
  }, [content]);
  return (
    <section
      className="markdown-preview"
      aria-label="Markdown 预览"
      aria-busy={updating}
    >
      <p className="preview-note">
        {updating ? '预览更新中 · ' : ''}HTML
        按文本显示；不加载图片，不执行图表或脚本。
      </p>
      {result.tree ? (
        renderNode(result.tree, 0)
      ) : (
        <div className="markdown-body">
          <p role="alert">{result.error}</p>
          <pre>{content}</pre>
        </div>
      )}
    </section>
  );
}
