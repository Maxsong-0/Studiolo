import { Component } from 'react';
import type { ReactNode } from 'react';
import '../styles/editor.css';

export class EditorTextBoundary extends Component<
  { content: string; children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  render() {
    // 出错说明套红色虚线框；安全纯文本与说明同在一个区块里，读屏与复制都不会漏掉草稿。
    return this.state.failed ? (
      <section className="editor-fallback">
        <p role="alert" className="editor-notice-error">
          编辑组件加载或渲染失败，已保留安全纯文本。
        </p>
        <pre>{this.props.content}</pre>
      </section>
    ) : (
      this.props.children
    );
  }
}
