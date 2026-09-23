import { Suspense } from 'react';
import type { ReactNode } from 'react';
import { EditorTextBoundary } from './EditorTextBoundary';
import '../styles/editor.css';

/** 加载与异常都只展示当前草稿的文本，不产生第二个可编辑状态。 */
export function EditorPane({
  content,
  label,
  children,
}: {
  content: string;
  label: string;
  children: ReactNode;
}) {
  return (
    <EditorTextBoundary content={content}>
      <Suspense
        fallback={
          <section className="editor-fallback" aria-label={`${label}加载状态`}>
            <p role="status">正在载入{label}，暂以只读文本显示。</p>
            <pre>{content}</pre>
          </section>
        }
      >
        {children}
      </Suspense>
    </EditorTextBoundary>
  );
}
