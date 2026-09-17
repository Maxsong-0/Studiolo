import { Component } from 'react';
import type { ReactNode } from 'react';
export class EditorTextBoundary extends Component<
  { content: string; children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  render() {
    return this.state.failed ? (
      <section>
        <p role="alert">编辑组件加载或渲染失败，已保留安全纯文本。</p>
        <pre>{this.props.content}</pre>
      </section>
    ) : (
      this.props.children
    );
  }
}
