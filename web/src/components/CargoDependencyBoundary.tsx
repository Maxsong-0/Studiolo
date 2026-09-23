import { Component } from 'react';
import type { ReactNode } from 'react';
import type { CargoDependencyGraph } from '../cargoDependencies';
import { CargoDependencyText } from './CargoDependencyText';
import '../styles/operations.css';
import '../styles/cargo-dependencies.css';

/**
 * 清单依赖视图的渲染边界：视图抛错时退回同一份图的只读文本（原因、全部包、方向、条件与来源）。
 * 版式同运作页其他块的文本回退；降级不是预算或数据问题，不用红色。
 */
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
      <div className="operation-fallback cargo-fallback">
        <h3>清单依赖图暂时无法显示，已切换为文本</h3>
        <p role="alert">原因：{this.state.error}</p>
        <CargoDependencyText graph={this.props.graph} />
      </div>
    );
  }
}
