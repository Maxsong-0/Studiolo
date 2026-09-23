import { Component } from 'react';
import type { ReactNode } from 'react';
import type { ArchitectureData } from '../architecture';
import type { OpenSource } from '../types';
import { Cell } from '../ui';
import { ArchitectureTextData } from './ArchitectureTextData';
import '../styles/operations.css';
import '../styles/architecture.css';

/** 架构图意外渲染失败时，整行换成三类声明的纯文本（原因与每条原文入口保留，不放图形）。 */
export class ArchitectureBoundary extends Component<
  {
    data: ArchitectureData;
    onOpen: OpenSource;
    children: ReactNode;
  },
  { error: string | null }
> {
  state = { error: null as string | null };
  static getDerivedStateFromError(error: unknown) {
    return {
      error: error instanceof Error ? error.message : '未知的架构视图错误',
    };
  }
  render() {
    if (this.state.error === null) return this.props.children;
    return (
      <Cell span={12} as="div" className="operation-fallback">
        <h3>架构视图暂时无法显示，已切换为文本</h3>
        <p role="alert">原因：{this.state.error}</p>
        <ArchitectureTextData
          data={this.props.data}
          onOpen={this.props.onOpen}
        />
      </Cell>
    );
  }
}
