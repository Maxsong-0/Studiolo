import { Component } from 'react';
import type { ReactNode } from 'react';
import type { ProjectOperationsData } from '../projectOperations';
import type { RustCallGraph } from '../rustCallSites';
import type { OpenSource } from '../types';
import { RustCallSitesText } from './RustCallSitesText';
import '../styles/operations.css';
import '../styles/rust-call-sites.css';

/**
 * 源码调用位置视图的渲染边界：视图抛错时退回同一份证据的只读文本（覆盖、上下文、位置、关联与反查）。
 * 版式同运作页其他块的文本回退；降级不是预算或数据问题，不用红色。
 */
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
      <div className="operation-fallback rust-fallback">
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
