import { Component } from 'react';
import type { ReactNode } from 'react';
import type { ArchitectureData } from '../architecture';
import type { OpenSource } from '../types';
import { ArchitectureTextData } from './ArchitectureTextData';

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
      <div className="operation-unavailable">
        <h3>架构视图暂时无法显示，已切换为文本</h3>
        <p role="alert">原因：{this.state.error}</p>
        <ArchitectureTextData
          data={this.props.data}
          onOpen={this.props.onOpen}
        />
      </div>
    );
  }
}
