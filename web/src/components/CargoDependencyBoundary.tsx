import { Component } from 'react';
import type { ReactNode } from 'react';
import type { CargoDependencyGraph } from '../cargoDependencies';
import { CargoDependencyText } from './CargoDependencyText';

export class CargoDependencyBoundary extends Component<
  {
    graph: CargoDependencyGraph;
    children: ReactNode;
  },
  { error: string | null }
> {
  state = { error: null as string | null };
  static getDerivedStateFromError(error: unknown) {
    return {
      error: error instanceof Error ? error.message : '未知的清单依赖视图错误',
    };
  }
  render() {
    if (this.state.error === null) return this.props.children;
    return (
      <div className="operation-unavailable">
        <h3>清单依赖图暂时无法显示，已切换为文本</h3>
        <p role="alert">原因：{this.state.error}</p>
        <CargoDependencyText graph={this.props.graph} />
      </div>
    );
  }
}
