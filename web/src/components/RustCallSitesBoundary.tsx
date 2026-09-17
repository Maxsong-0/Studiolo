import { Component } from 'react';
import type { ReactNode } from 'react';
import type { ProjectOperationsData } from '../projectOperations';
import type { RustCallGraph } from '../rustCallSites';
import type { OpenSource } from '../types';
import { RustCallSitesText } from './RustCallSitesText';

export class RustCallSitesBoundary extends Component<
  {
    graph: RustCallGraph;
    data: ProjectOperationsData;
    onOpen: OpenSource;
    children: ReactNode;
  },
  { error: string | null }
> {
  state = { error: null as string | null };
  static getDerivedStateFromError(error: unknown) {
    return {
      error:
        error instanceof Error ? error.message : '未知的源码调用位置视图错误',
    };
  }
  render() {
    if (this.state.error === null) return this.props.children;
    return (
      <div className="operation-unavailable">
        <h3>源码调用位置视图暂时无法显示，已切换为文本</h3>
        <p role="alert">原因：{this.state.error}</p>
        <RustCallSitesText
          graph={this.props.graph}
          data={this.props.data}
          onOpen={this.props.onOpen}
        />
      </div>
    );
  }
}
